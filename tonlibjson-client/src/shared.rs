use std::sync::Arc;
use std::task::{Context, Poll};
use std::sync::Mutex;
use tower::{Layer, Service};
use tower::load::Load;

#[derive(Default)]
pub struct SharedLayer;

impl<S> Layer<S> for SharedLayer {
    type Service = SharedService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SharedService::new(inner)
    }
}

pub struct SharedService<S> {
    inner: Arc<Mutex<S>>
}

impl<S> Clone for SharedService<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone()
        }
    }
}

impl<S> SharedService<S> {
    pub fn new(inner: S) -> Self {
        Self { inner: Arc::new(Mutex::new(inner)) }
    }
}

impl<S, Req> Service<Req> for SharedService<S>
    where S : Service<Req> {
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self.inner.try_lock() {
            Ok(mut lock) => {
                lock.poll_ready(cx)
            }
            Err(_) => {
                Poll::Pending
            }
        }
    }

    fn call(&mut self, req: Req) -> Self::Future {
        self.inner.lock().expect("call ready first").call(req)
    }
}

impl<S> Load for SharedService<S> where S : Load {
    type Metric = S::Metric;

    fn load(&self) -> Self::Metric {
        self.inner.lock().unwrap().load()
    }
}
