use jsonrpsee::{core::RpcResult as Result, proc_macros::rpc};
use reth_primitives::{
    filter::Filter, Address, BlockHash, BlockId, BlockNumberOrTag, Bytes, H256, U256, U64,
};
use reth_rpc_types::{
    engine::{
        ExecutionPayload, ExecutionPayloadBodies, ExecutionPayloadEnvelope, ForkchoiceState,
        ForkchoiceUpdated, PayloadAttributes, PayloadId, PayloadStatus, TransitionConfiguration,
    },
    state::StateOverride,
    CallRequest, Log, RichBlock, SyncStatus,
};

#[cfg_attr(not(feature = "client"), rpc(server))]
#[cfg_attr(feature = "client", rpc(server, client))]
pub trait EngineApi {
    /// See also <https://github.com/ethereum/execution-apis/blob/6709c2a795b707202e93c4f2867fa0bf2640a84f/src/engine/paris.md#engine_newpayloadv1>
    /// Caution: This should not accept the `withdrawals` field
    #[method(name = "engine_newPayloadV1")]
    async fn new_payload_v1(&self, payload: ExecutionPayload) -> Result<PayloadStatus>;

    /// See also <https://github.com/ethereum/execution-apis/blob/6709c2a795b707202e93c4f2867fa0bf2640a84f/src/engine/shanghai.md#engine_newpayloadv2>
    #[method(name = "engine_newPayloadV2")]
    async fn new_payload_v2(&self, payload: ExecutionPayload) -> Result<PayloadStatus>;

    /// See also <https://github.com/ethereum/execution-apis/blob/6709c2a795b707202e93c4f2867fa0bf2640a84f/src/engine/paris.md#engine_forkchoiceupdatedv1>
    ///
    /// Caution: This should not accept the `withdrawals` field
    #[method(name = "engine_forkchoiceUpdatedV1")]
    async fn fork_choice_updated_v1(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<PayloadAttributes>,
    ) -> Result<ForkchoiceUpdated>;

    /// See also <https://github.com/ethereum/execution-apis/blob/6709c2a795b707202e93c4f2867fa0bf2640a84f/src/engine/shanghai.md#engine_forkchoiceupdatedv2>
    #[method(name = "engine_forkchoiceUpdatedV2")]
    async fn fork_choice_updated_v2(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<PayloadAttributes>,
    ) -> Result<ForkchoiceUpdated>;

    /// See also <https://github.com/ethereum/execution-apis/blob/6709c2a795b707202e93c4f2867fa0bf2640a84f/src/engine/paris.md#engine_getpayloadv1>
    ///
    /// Returns the most recent version of the payload that is available in the corresponding
    /// payload build process at the time of receiving this call.
    ///
    /// Caution: This should not return the `withdrawals` field
    ///
    /// Note:
    /// > Client software MAY stop the corresponding build process after serving this call.
    #[method(name = "engine_getPayloadV1")]
    async fn get_payload_v1(&self, payload_id: PayloadId) -> Result<ExecutionPayload>;

    /// See also <https://github.com/ethereum/execution-apis/blob/6709c2a795b707202e93c4f2867fa0bf2640a84f/src/engine/shanghai.md#engine_getpayloadv2>
    ///
    /// Returns the most recent version of the payload that is available in the corresponding
    /// payload build process at the time of receiving this call. Note:
    /// > Client software MAY stop the corresponding build process after serving this call.
    #[method(name = "engine_getPayloadV2")]
    async fn get_payload_v2(&self, payload_id: PayloadId) -> Result<ExecutionPayloadEnvelope>;

    /// See also <https://github.com/ethereum/execution-apis/blob/6452a6b194d7db269bf1dbd087a267251d3cc7f8/src/engine/shanghai.md#engine_getpayloadbodiesbyhashv1>
    #[method(name = "engine_getPayloadBodiesByHashV1")]
    async fn get_payload_bodies_by_hash_v1(
        &self,
        block_hashes: Vec<BlockHash>,
    ) -> Result<ExecutionPayloadBodies>;

    /// See also <https://github.com/ethereum/execution-apis/blob/6452a6b194d7db269bf1dbd087a267251d3cc7f8/src/engine/shanghai.md#engine_getpayloadbodiesbyrangev1>
    #[method(name = "engine_getPayloadBodiesByRangeV1")]
    async fn get_payload_bodies_by_range_v1(
        &self,
        start: U64,
        count: U64,
    ) -> Result<ExecutionPayloadBodies>;

    /// See also <https://github.com/ethereum/execution-apis/blob/6709c2a795b707202e93c4f2867fa0bf2640a84f/src/engine/paris.md#engine_exchangetransitionconfigurationv1>
    #[method(name = "engine_exchangeTransitionConfigurationV1")]
    async fn exchange_transition_configuration(
        &self,
        transition_configuration: TransitionConfiguration,
    ) -> Result<TransitionConfiguration>;

    /// See also <https://github.com/ethereum/execution-apis/blob/6452a6b194d7db269bf1dbd087a267251d3cc7f8/src/engine/common.md#capabilities>
    #[method(name = "engine_exchangeCapabilities")]
    async fn exchange_capabilities(&self, capabilities: Vec<String>) -> Result<Vec<String>>;
}

/// A subset of the ETH rpc interface: <https://ethereum.github.io/execution-apis/api-documentation/>
///
/// Specifically for the engine auth server: <https://github.com/ethereum/execution-apis/blob/main/src/engine/common.md#underlying-protocol>
#[cfg_attr(not(feature = "client"), rpc(server))]
#[cfg_attr(feature = "client", rpc(server, client))]
#[async_trait]
pub trait EngineEthApi {
    /// Returns an object with data about the sync status or false.
    #[method(name = "eth_syncing")]
    fn syncing(&self) -> Result<SyncStatus>;

    /// Returns the chain ID of the current network.
    #[method(name = "eth_chainId")]
    async fn chain_id(&self) -> Result<Option<U64>>;

    /// Returns the number of most recent block.
    #[method(name = "eth_blockNumber")]
    fn block_number(&self) -> Result<U256>;

    /// Executes a new message call immediately without creating a transaction on the block chain.
    #[method(name = "eth_call")]
    async fn call(
        &self,
        request: CallRequest,
        block_number: Option<BlockId>,
        state_overrides: Option<StateOverride>,
    ) -> Result<Bytes>;

    /// Returns code at a given address at given block number.
    #[method(name = "eth_getCode")]
    async fn get_code(&self, address: Address, block_number: Option<BlockId>) -> Result<Bytes>;

    /// Returns information about a block by hash.
    #[method(name = "eth_getBlockByHash")]
    async fn block_by_hash(&self, hash: H256, full: bool) -> Result<Option<RichBlock>>;

    /// Returns information about a block by number.
    #[method(name = "eth_getBlockByNumber")]
    async fn block_by_number(
        &self,
        number: BlockNumberOrTag,
        full: bool,
    ) -> Result<Option<RichBlock>>;

    /// Sends signed transaction, returning its hash.
    #[method(name = "eth_sendRawTransaction")]
    async fn send_raw_transaction(&self, bytes: Bytes) -> Result<H256>;

    /// Returns logs matching given filter object.
    #[method(name = "eth_getLogs")]
    async fn logs(&self, filter: Filter) -> Result<Vec<Log>>;
}
