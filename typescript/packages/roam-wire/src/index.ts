// Roam wire protocol types and utilities
//
// This package contains Roam-specific wire protocol types including
// RPC error handling that follows the RAPACE specification.

export {
  RpcError,
  RpcErrorCode,
  decodeRpcResult,
  decodeUserError,
} from "./rpc_error.ts";
