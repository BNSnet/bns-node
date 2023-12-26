#![warn(missing_docs)]
//! FFI backend behaviour implementation
//! =================================
//！
use std::ffi::c_char;
use std::ffi::CString;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::runtime::Runtime;

use crate::backend::types::BackendMessage;
use crate::backend::types::MessageEndpoint;
use crate::error::Result;
use crate::prelude::MessagePayload;
use crate::provider::ffi::ProviderPtr;
use crate::provider::ffi::ProviderWithRuntime;
use crate::provider::Provider;

/// Context for handling backend behaviour
#[repr(C)]
#[derive(Clone)]
pub struct FFIBackendBehaviour {
    paintext_message_handler: Option<
        Box<
            extern "C" fn(
                *const FFIBackendBehaviour,
                *const ProviderPtr,
                *const c_char,
                *const c_char,
            ) -> (),
        >,
    >,
    service_message_handler: Option<
        Box<
            extern "C" fn(
                *const FFIBackendBehaviour,
                *const ProviderPtr,
                *const c_char,
                *const c_char,
            ) -> (),
        >,
    >,
    extension_message_handler: Option<
        Box<
            extern "C" fn(
                *const FFIBackendBehaviour,
                *const ProviderPtr,
                *const c_char,
                *const c_char,
            ) -> (),
        >,
    >,
    runtime: Option<Arc<Runtime>>,
}

impl FFIBackendBehaviour {
    pub(crate) fn with_runtime(&mut self, rt: Arc<Runtime>) {
        self.runtime = Some(rt.clone())
    }
}

/// Backend behaviour for FFI
#[no_mangle]
pub extern "C" fn new_ffi_backend_behaviour(
    paintext_message_handler: Option<
        extern "C" fn(
            *const FFIBackendBehaviour,
            *const ProviderPtr,
            *const c_char,
            *const c_char,
        ) -> (),
    >,
    service_message_handler: Option<
        extern "C" fn(
            *const FFIBackendBehaviour,
            *const ProviderPtr,
            *const c_char,
            *const c_char,
        ) -> (),
    >,
    extension_message_handler: Option<
        extern "C" fn(
            *const FFIBackendBehaviour,
            *const ProviderPtr,
            *const c_char,
            *const c_char,
        ) -> (),
    >,
) -> FFIBackendBehaviour {
    FFIBackendBehaviour {
        paintext_message_handler: paintext_message_handler.map(|c| Box::new(c)),
        service_message_handler: service_message_handler.map(|c| Box::new(c)),
        extension_message_handler: extension_message_handler.map(|c| Box::new(c)),
        runtime: None,
    }
}

macro_rules! handle_backend_message {
    ($self:ident, $provider:ident, $handler:ident, $payload: ident, $message:ident) => {
        if let Some(handler) = &$self.$handler {
            let provider_with_runtime = ProviderWithRuntime::new(
                $provider.clone(),
                $self.runtime.clone().expect("Runtime is not found").clone(),
            );
            let provider_ptr: ProviderPtr = (&provider_with_runtime).into();
            let payload = serde_json::to_string(&$payload)?;
            let message = serde_json::to_string(&$message)?;
            let payload = CString::new(payload)?;
            let message = CString::new(message)?;
            handler(
                $self as *const FFIBackendBehaviour,
                &provider_ptr as *const ProviderPtr,
                payload.as_ptr(),
                message.as_ptr(),
            );
        }
    };
}

#[async_trait]
impl MessageEndpoint<BackendMessage> for FFIBackendBehaviour {
    async fn on_message(
        &self,
        provider: Arc<Provider>,
        payload: &MessagePayload,
        msg: &BackendMessage,
    ) -> Result<()> {
        match msg {
            BackendMessage::PlainText(m) => {
                handle_backend_message!(self, provider, paintext_message_handler, payload, m)
            }
            BackendMessage::Extension(m) => {
                handle_backend_message!(self, provider, extension_message_handler, payload, m)
            }
            BackendMessage::ServiceMessage(m) => {
                handle_backend_message!(self, provider, service_message_handler, payload, m)
            }
        }
        Ok(())
    }
}
