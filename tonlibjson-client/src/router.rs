use std::collections::HashMap;
use std::future::{Ready, ready};
use std::hash::Hash;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use tower::discover::{Change, Discover};
use tower::Service;
use ton_client_utils::router::{Route, RouterError};
use crate::cursor_client::CursorClient;
use crate::error::Error;

pub(crate) trait Routable {
    fn route(&self) -> Route { Route::Latest }
}

pub(crate) struct Router<D>
    where
        D: Discover<Service=CursorClient, Error = anyhow::Error> + Unpin,
        D::Key: Eq + Hash,
{
    discover: D,
    services: HashMap<D::Key, CursorClient>
}

impl<D> Router<D>
    where
        D: Discover<Service=CursorClient, Error = anyhow::Error> + Unpin,
        D::Key: Eq + Hash,
{
    pub(crate) fn new(discover: D) -> Self {
        metrics::describe_counter!("ton_router_miss_count", "Count of misses in router");
        metrics::describe_counter!("ton_router_fallback_hit_count", "Count of fallback request hits in router");
        metrics::describe_counter!("ton_router_delayed_count", "Count of delayed requests in router");
        metrics::describe_counter!("ton_router_delayed_hit_count", "Count of delayed request hits in router");
        metrics::describe_counter!("ton_router_delayed_miss_count", "Count of delayed request misses in router");

        Router { discover, services: Default::default() }
    }

    fn update_pending_from_discover(&mut self, cx: &mut Context<'_>, ) -> Poll<Option<Result<(), anyhow::Error>>> {
        loop {
            match ready!(Pin::new(&mut self.discover).poll_discover(cx)).transpose()? {
                None => return Poll::Ready(None),
                Some(Change::Remove(key)) => {
                    self.services.remove(&key);
                }
                Some(Change::Insert(key, svc)) => {
                    self.services.insert(key, svc);
                }
            }
        }
    }
}

impl<D> Service<&Route> for Router<D>
    where
        D: Discover<Service=CursorClient, Error = anyhow::Error> + Unpin,
        D::Key: Eq + Hash,
{
    type Response = Vec<CursorClient>;
    type Error = Error;
    type Future = Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let _ = self.update_pending_from_discover(cx)
            .map_err(Error::Custom)?;

        if self.services.values().any(|s| s.edges_defined()) {
            Poll::Ready(Ok(()))
        } else {
            cx.waker().wake_by_ref();

            Poll::Pending
        }
    }

    fn call(&mut self, req: &Route) -> Self::Future {
        match req.choose(self.services.values()) {
            Ok(services) => ready(Ok(services.into_iter().cloned().collect())),
            Err(RouterError::RouteUnknown) => {
                metrics::counter!("ton_router_miss_count").increment(1);

                ready(
                    Route::Latest.choose(self.services.values())
                        .map(|services| services.into_iter().cloned().collect())
                        .map_err(Error::Router)
                )
            },
            Err(RouterError::RouteNotAvailable) => {
                metrics::counter!("ton_router_delayed_count").increment(1);

                ready(Err(Error::Router(RouterError::RouteNotAvailable)))
            },
        }
    }
}
