use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use anyhow::anyhow;
use futures::TryFutureExt;
use serde_json::Value;
use tower::{BoxError, Service, ServiceExt};
use tower::buffer::Buffer;
use crate::session::SessionRequest::{Atomic, RunGetMethod};
use crate::{Client, Request};
use crate::block::{SmcInfo, SmcLoad, SmcMethodId, SmcRunGetMethod, SmcStack};

#[derive(Clone)]
pub enum SessionRequest {
    RunGetMethod { address: String, method: String, stack: SmcStack },
    Atomic(Request)
}

impl From<Request> for SessionRequest {
    fn from(req: Request) -> Self {
        Atomic(req)
    }
}

pub struct SessionClient {
    client: Buffer<Client, Request>
}

impl SessionClient {
    pub fn new(client: Client) -> Self {
        Self {
            client: Buffer::new(client, 10000)
        }
    }
}

impl Service<SessionRequest> for SessionClient {
    type Response = Value;
    type Error = anyhow::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.client.poll_ready(cx).map_err(|e| anyhow!(e))
    }

    fn call(&mut self, req: SessionRequest) -> Self::Future {
        match req {
            Atomic(req) => Box::pin(self.client.call(req).map_err(|e| anyhow!(e))),
            RunGetMethod { address, method, stack} => {
                let mut this = self.client.clone();
                Box::pin(async move {
                    let req = SmcLoad::new(address);
                    let resp = this.ready().await?
                        .call(Request::new(&req)?).await?;

                    let info = serde_json::from_value::<SmcInfo>(resp)?;

                    let req = SmcRunGetMethod::new(
                        info.id,
                        SmcMethodId::Name {name: method},
                        stack
                    );

                    let resp = this.ready().await?
                        .call(Request::new(&req)?).await?;

                    Ok(resp)
                }.map_err(|e: BoxError| anyhow!(e)))
            }
        }
    }
}
