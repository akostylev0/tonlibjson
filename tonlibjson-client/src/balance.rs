use std::{pin::Pin, task::{Context, Poll}};
use std::collections::HashMap;
use std::future::{Future, Ready};
use futures::{ready, StreamExt, TryFutureExt, FutureExt};
use tokio::select;
use tokio_stream::StreamMap;
use tokio_stream::wrappers::WatchStream;
use tower::{Service, ServiceExt};
use tower::discover::{Change, Discover, ServiceList};
use anyhow::anyhow;
use derive_new::new;
use itertools::Itertools;
use crate::block::{BlockHeader, BlockIdExt, BlocksGetShards, BlocksLookupBlock, BlocksShards, GetMasterchainInfo, MasterchainInfo};
use crate::cursor_client::{CursorClient, InnerClient};
use crate::discover::CursorClientDiscover;
use crate::error::ErrorService;
use crate::request::{Routable, Callable, Specialized};

#[derive(Debug, Clone, Copy)]
pub enum BlockCriteria {
    Seqno(i32),
    LogicalTime(i64)
}

#[derive(Debug, Clone, Copy)]
pub enum Route {
    Any,
    Block { chain: i32, criteria: BlockCriteria },
    Latest { chain: i32 }
}

impl Route {
    pub fn choose<'a, T : Iterator<Item=&'a CursorClient>>(&self, services: T) -> Vec<CursorClient> {
        match self {
            Route::Any => { services.cloned().collect() },
            Route::Block { chain, criteria} => {
                services
                    .filter_map(|s| s.headers(*chain).map(|m| (s, m)))
                    .filter(|(_, (first_block, last_block))| { match criteria {
                        BlockCriteria::LogicalTime(lt) => first_block.start_lt <= *lt && *lt <= last_block.end_lt,
                        BlockCriteria::Seqno(seqno) => first_block.id.seqno <= *seqno && *seqno <= last_block.id.seqno
                    }})
                    .map(|(s, _)| s)
                    .cloned()
                    .collect()
            },
            Route::Latest { chain } => {
                fn last_seqno(client: &CursorClient, chain: &i32) -> Option<i32> {
                    client.headers(*chain).map(|(_, l)| l.id.seqno)
                }

                let groups = services
                    .filter_map(|s| last_seqno(s, chain).map(|seqno| (s, seqno)))
                    .sorted_unstable_by_key(|(_, seqno)| -seqno)
                    .group_by(|(_, seqno)| *seqno);

                let mut idxs = vec![];
                for (_, group) in &groups {
                    idxs = group.collect();

                    // we need at least 3 nodes in group
                    if idxs.len() > 2 {
                        break;
                    }
                }

                idxs.into_iter().map(|(s, _)| s).cloned().collect()
            }
        }
    }
}

pub struct Router {
    discover: CursorClientDiscover,
    services: HashMap<String, CursorClient>,
    pub first_headers: BlockChannel,
    pub last_headers: BlockChannel
}

impl Router {
    pub fn new(discover: CursorClientDiscover) -> Self {
        Router {
            discover,
            services: HashMap::new(),
            first_headers: BlockChannel::new(BlockChannelMode::Min),
            last_headers: BlockChannel::new(BlockChannelMode::Max)
        }
    }

    fn update_pending_from_discover(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<(), anyhow::Error>>> {
        loop {
            match ready!(Pin::new(&mut self.discover).poll_discover(cx))
                .transpose()
                .map_err(|e| anyhow!(e))?
            {
                None => return Poll::Ready(None),
                Some(Change::Remove(key)) => {
                    self.services.remove(&key);
                    self.first_headers.remove(&key);
                    self.last_headers.remove(&key);
                }
                Some(Change::Insert(key, svc)) => {
                    self.first_headers.insert(key.clone(), svc.first_block_receiver());
                    self.last_headers.insert(key.clone(), svc.last_block_rx.clone());
                    self.services.insert(key, svc);
                }
            }
        }
    }
}

#[derive(new)]
pub struct Balance { router: Router }

impl Service<&Route> for Router {
    type Response = Vec<CursorClient>;
    type Error = anyhow::Error;
    type Future = Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let _ = self.update_pending_from_discover(cx)?;

        if self.services.values().any(|s| s.headers(-1).is_some()) {
            Poll::Ready(Ok(()))
        } else {
            cx.waker().wake_by_ref();

            Poll::Pending
        }
    }

    fn call(&mut self, req: &Route) -> Self::Future {
        let services = req.choose(self.services.values());

        let response = if services.is_empty() {
            Err(anyhow!("no services available for {:?}", req))
        } else {
            Ok(services)
        };

        std::future::ready(response)
    }
}

impl<R> Service<R> for Balance where R: Routable + Callable<InnerClient> + Clone {
    type Response = R::Response;
    type Error = anyhow::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.router.poll_ready(cx)
    }

    fn call(&mut self, req: R) -> Self::Future {
        self.router
            .call(&req.route())
            .and_then(|svc| ErrorService::new(tower::balance::p2c::Balance::new(ServiceList::new::<R>(svc)))
                .oneshot(req))
            .boxed()
    }
}

impl Service<Specialized<GetMasterchainInfo>> for Balance {
    type Response = MasterchainInfo;
    type Error = anyhow::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.router.poll_ready(cx)
    }

    fn call(&mut self, req: Specialized<GetMasterchainInfo>) -> Self::Future {
        self.router
            .call(&req.route())
            .and_then(|svc| ErrorService::new(tower::balance::p2c::Balance::new(
                ServiceList::new::<Specialized<GetMasterchainInfo>>(svc))).oneshot(req))
            .boxed()
    }
}

// TODO[akostylev0] generics
impl Service<Specialized<BlocksGetShards>> for Balance {
    type Response = BlocksShards;
    type Error = anyhow::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.router.poll_ready(cx)
    }

    fn call(&mut self, req: Specialized<BlocksGetShards>) -> Self::Future {
        self.router
            .call(&req.route())
            .and_then(|svc| ErrorService::new(tower::balance::p2c::Balance::new(
                ServiceList::new::<Specialized<BlocksGetShards>>(svc))).oneshot(req))
            .boxed()
    }
}

// TODO[akostylev0] generics
impl Service<Specialized<BlocksLookupBlock>> for Balance {
    type Response = BlockIdExt;
    type Error = anyhow::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.router.poll_ready(cx)
    }

    fn call(&mut self, req: Specialized<BlocksLookupBlock>) -> Self::Future {
        self.router
            .call(&req.route())
            .and_then(|svc| ErrorService::new(tower::balance::p2c::Balance::new(
                ServiceList::new::<Specialized<BlocksLookupBlock>>(svc))).oneshot(req))
            .boxed()
    }
}

type BlockChannelItem = (BlockHeader, BlockHeader);

enum BlockChannelChange {
    Insert { key: String, watcher: tokio::sync::watch::Receiver<Option<BlockChannelItem>>},
    Remove { key: String }
}

pub struct BlockChannel {
    changes: tokio::sync::mpsc::UnboundedSender<BlockChannelChange>,
    joined: tokio::sync::broadcast::Receiver<BlockChannelItem>
}

pub enum BlockChannelMode { Max, Min }

impl BlockChannel {
    pub fn new(mode: BlockChannelMode) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<BlockChannelChange>();
        let (tj, rj) = tokio::sync::broadcast::channel::<BlockChannelItem>(256);

        tokio::spawn(async move {
            let mut stream_map = StreamMap::new();
            let mut last_seqno = 0;

            loop {
                select! {
                    Some(change) = rx.recv() => {
                        match change {
                            BlockChannelChange::Insert { key, watcher } => { stream_map.insert(key, WatchStream::from_changes(watcher)); },
                            BlockChannelChange::Remove { key } => { stream_map.remove(&key); }
                        }
                    },
                    Some((_, Some((master, worker)))) = stream_map.next() => {
                       if last_seqno == 0 || match mode {
                            BlockChannelMode::Max => { master.id.seqno > last_seqno },
                            BlockChannelMode::Min => { master.id.seqno < last_seqno }
                        } {
                            last_seqno = master.id.seqno;

                            let _ = tj.send((master, worker));
                        }
                    }
                };
            }
        });

        Self {
            changes: tx,
            joined: rj
        }
    }

    pub fn insert(&self, key: String, watcher: tokio::sync::watch::Receiver<Option<BlockChannelItem>>) {
        let _ = self.changes.send(BlockChannelChange::Insert { key, watcher });
    }

    pub fn remove(&self, key: &str) {
        let _ = self.changes.send(BlockChannelChange::Remove { key: key.to_owned() });
    }

    pub fn receiver(&self) -> tokio::sync::broadcast::Receiver<BlockChannelItem> {
        self.joined.resubscribe()
    }
}
