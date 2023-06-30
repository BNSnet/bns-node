//! A bunch of wrap errors.
use crate::prelude::jsonrpc_core;
use crate::prelude::rings_core;

/// A wrap `Result` contains custom errors.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
/// Errors enum mapping global custom errors.
pub enum Error {
    #[error("Connect remote rpc server failed: {0}.")]
    RemoteRpcError(String),
    #[error("Pending Transport error: {0}.")]
    PendingTransport(rings_core::error::Error),
    #[error("Transport not found.")]
    TransportNotFound,
    #[error("Create Transport error: {0}.")]
    NewTransportError(rings_core::error::Error),
    #[error("Close Transport error: {0}.")]
    CloseTransportError(rings_core::error::Error),
    #[error("Decode error.")]
    DecodeError,
    #[error("Encode error.")]
    EncodeError,
    #[error("WASM compile error: {0}")]
    WasmCompileError(String),
    #[error("BackendMessage RwLock Error")]
    WasmBackendMessageRwLockError,
    #[error("WASM instantiation error.")]
    WasmInstantiationError,
    #[error("WASM export error.")]
    WasmExportError,
    #[error("WASM runtime error: {0}")]
    WasmRuntimeError(String),
    #[error("WASM global memory mutex error.")]
    WasmGlobalMemoryLockError,
    #[error("WASM failed to load file.")]
    WasmFailedToLoadFile,
    #[error("Create offer info failed: {0}.")]
    CreateOffer(rings_core::error::Error),
    #[error("Answer offer info failed: {0}.")]
    AnswerOffer(rings_core::error::Error),
    #[error("Accept answer info failed: {0}.")]
    AcceptAnswer(rings_core::error::Error),
    #[error("Invalid transport id.")]
    InvalidTransportId,
    #[error("Invalid did.")]
    InvalidDid,
    #[error("Invalid method.")]
    InvalidMethod,
    #[error("Internal error.")]
    InternalError,
    #[error("Connect error, {0}")]
    ConnectError(rings_core::error::Error),
    #[error("Send message error: {0}")]
    SendMessage(rings_core::error::Error),
    #[error("No Permission")]
    NoPermission,
    #[error("vnode action error: {0}")]
    VNodeError(rings_core::error::Error),
    #[error("service register action error: {0}")]
    ServiceRegisterError(rings_core::error::Error),
    #[error("JsError: {0}")]
    JsError(String),
    #[error("Invalid http request: {0}")]
    HttpRequestError(String),
    #[error("Invalid message")]
    InvalidMessage,
    #[error("Invalid data")]
    InvalidData,
    #[error("Invalid service")]
    InvalidService,
    #[error("Invalid address")]
    InvalidAddress,
    #[error("Invalid auth data")]
    InvalidAuthData,
    #[error("Storage Error: {0}")]
    Storage(rings_core::error::Error),
    #[error("Swarm Error: {0}")]
    Swarm(rings_core::error::Error),
    #[error("Create File Error: {0}")]
    CreateFileError(String),
    #[error("Open File Error: {0}")]
    OpenFileError(String),
    #[error("acquire lock failed")]
    Lock,
    #[error("invalid headers")]
    InvalidHeaders,
    #[error("serde json error: {0}")]
    SerdeJsonError(#[from] serde_json::Error),
    #[error("verify error: {0}")]
    VerifyError(String),
}

impl Error {
    pub fn code(&self) -> i64 {
        let code = match self {
            Error::RemoteRpcError(_) => 1,
            Error::ConnectError(_) => 1,
            Error::HttpRequestError(_) => 1,
            Error::PendingTransport(_) => 2,
            Error::TransportNotFound => 3,
            Error::NewTransportError(_) => 4,
            Error::CloseTransportError(_) => 5,
            Error::EncodeError => 6,
            Error::DecodeError => 7,
            Error::CreateOffer(_) => 8,
            Error::AnswerOffer(_) => 9,
            Error::AcceptAnswer(_) => 10,
            Error::InvalidTransportId => 11,
            Error::InvalidDid => 12,
            Error::InvalidMethod => 13,
            Error::SendMessage(_) => 14,
            Error::NoPermission => 15,
            Error::VNodeError(_) => 16,
            Error::ServiceRegisterError(_) => 17,
            Error::InvalidData => 18,
            Error::InvalidMessage => 19,
            Error::InvalidService => 20,
            Error::InvalidAddress => 21,
            Error::InvalidAuthData => 22,
            Error::InvalidHeaders => 23,
            Error::SerdeJsonError(_) => 24,
            Error::WasmCompileError(_) => 25,
            Error::WasmInstantiationError => 26,
            Error::WasmExportError => 27,
            Error::WasmRuntimeError(_) => 28,
            Error::WasmGlobalMemoryLockError => 29,
            Error::WasmFailedToLoadFile => 30,
            Error::WasmBackendMessageRwLockError => 31,
            Error::InternalError => 0,
            Error::CreateFileError(_) => 0,
            Error::OpenFileError(_) => 0,
            Error::JsError(_) => 0,
            Error::Swarm(_) => 0,
            Error::Storage(_) => 0,
            Error::VerifyError(_) => 0,
            Error::Lock => 0,
        };
        -32000 - code
    }
}

impl From<Error> for jsonrpc_core::Error {
    fn from(e: Error) -> Self {
        Self {
            code: jsonrpc_core::ErrorCode::ServerError(e.code()),
            message: e.to_string(),
            data: None,
        }
    }
}

impl From<crate::prelude::rings_rpc::error::Error> for Error {
    fn from(e: crate::prelude::rings_rpc::error::Error) -> Self {
        match e {
            rings_rpc::error::Error::DecodeError => Error::DecodeError,
            rings_rpc::error::Error::EncodeError => Error::EncodeError,
            rings_rpc::error::Error::InvalidMethod => Error::InvalidMethod,
            rings_rpc::error::Error::RpcError(v) => Error::RemoteRpcError(v.to_string()),
            rings_rpc::error::Error::InvalidSignature => Error::InvalidData,
            rings_rpc::error::Error::InvalidHeaders => Error::InvalidHeaders,
        }
    }
}
