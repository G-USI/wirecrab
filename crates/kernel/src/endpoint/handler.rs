use crate::utils::structs::*;

use core::future::Future;

#[derive(Debug, Clone, Default)]
pub struct MessageContext {
    pub headers: BTreeMap<String, String>,
    pub correlation_id: Option<String>,
    pub reply_to: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Debug)]
pub enum HandlerError {
    Transient(AnyhowError),
    Reject(AnyhowError),
}

pub trait Handler<T>: ThreadSafe {
    type Future: Future<Output = Result<(), HandlerError>> + Send;
    fn handle(&self, payload: T, ctx: &MessageContext) -> Self::Future;
}

impl<T, F, Fut> Handler<T> for F
where
    F: Fn(T, &MessageContext) -> Fut + ThreadSafe,
    Fut: Future<Output = Result<(), HandlerError>> + Send,
{
    type Future = Fut;
    fn handle(&self, payload: T, ctx: &MessageContext) -> Self::Future {
        (self)(payload, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_context_default() {
        let ctx = MessageContext::default();
        assert!(ctx.headers.is_empty());
        assert!(ctx.correlation_id.is_none());
        assert!(ctx.reply_to.is_none());
        assert!(ctx.content_type.is_none());
    }

    #[test]
    fn handler_error_transient_construction() {
        let err = HandlerError::Transient(anyhow::anyhow!("network blip"));
        assert!(matches!(err, HandlerError::Transient(_)));
    }

    #[test]
    fn handler_error_reject_construction() {
        let err = HandlerError::Reject(anyhow::anyhow!("bad payload"));
        assert!(matches!(err, HandlerError::Reject(_)));
    }

    #[tokio::test]
    async fn handler_closure_ok_satisfies_trait() {
        let h = |_payload: i32, _ctx: &MessageContext| async move { Ok::<(), HandlerError>(()) };
        let ctx = MessageContext::default();
        let res = Handler::handle(&h, 42i32, &ctx).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn handler_closure_returns_transient() {
        let h = |_payload: i32, _ctx: &MessageContext| async move {
            Err::<(), HandlerError>(HandlerError::Transient(anyhow::anyhow!("retry me")))
        };
        let ctx = MessageContext::default();
        let res = Handler::handle(&h, 1i32, &ctx).await;
        assert!(matches!(res.unwrap_err(), HandlerError::Transient(_)));
    }

    #[tokio::test]
    async fn handler_closure_returns_reject() {
        let h = |_payload: i32, _ctx: &MessageContext| async move {
            Err::<(), HandlerError>(HandlerError::Reject(anyhow::anyhow!("nope")))
        };
        let ctx = MessageContext::default();
        let res = Handler::handle(&h, 1i32, &ctx).await;
        assert!(matches!(res.unwrap_err(), HandlerError::Reject(_)));
    }
}
