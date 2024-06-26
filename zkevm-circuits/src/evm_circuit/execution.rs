use super::{
    param::{
        BLOCK_TABLE_LOOKUPS, BYTECODE_TABLE_LOOKUPS, CHUNK_CTX_TABLE_LOOKUPS, COPY_TABLE_LOOKUPS,
        EXP_TABLE_LOOKUPS, FIXED_TABLE_LOOKUPS, KECCAK_TABLE_LOOKUPS, N_COPY_COLUMNS,
        N_PHASE1_COLUMNS, N_U16_LOOKUPS, N_U8_LOOKUPS, RW_TABLE_LOOKUPS, SIG_TABLE_LOOKUPS,
        TX_TABLE_LOOKUPS,
    },
    step::HasExecutionState,
    util::{instrumentation::Instrument, CachedRegion, StoredExpression},
};
use crate::{
    evm_circuit::{
        param::{EVM_LOOKUP_COLS, MAX_STEP_HEIGHT, N_PHASE2_COLUMNS, STEP_WIDTH},
        step::{ExecutionState, Step},
        table::Table,
        util::{
            constraint_builder::{
                BaseConstraintBuilder, ConstrainBuilderCommon, EVMConstraintBuilder,
            },
            evaluate_expression, rlc,
        },
        witness::{Block, Call, Chunk, ExecStep, Transaction},
    },
    table::{chunk_ctx_table::ChunkCtxFieldTag, LookupTable},
    util::{
        cell_manager::{CMFixedWidthStrategy, CellManager, CellType},
        Challenges, Expr,
    },
};
use bus_mapping::{circuit_input_builder::FeatureConfig, operation::Target};
use eth_types::{evm_unimplemented, Field};

use gadgets::{is_zero::IsZeroConfig, util::not};
use halo2_proofs::{
    circuit::{Layouter, Region, Value},
    plonk::{
        Advice, Column, ConstraintSystem, Error, Expression, FirstPhase, Fixed, SecondPhase,
        Selector, ThirdPhase, VirtualCells,
    },
    poly::Rotation,
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    iter,
};
use strum::IntoEnumIterator;

mod add_sub;
mod addmod;
mod address;
mod balance;
mod begin_chunk;
mod begin_tx;
mod bitwise;
mod block_ctx;
mod blockhash;
mod byte;
mod calldatacopy;
mod calldataload;
mod calldatasize;
mod caller;
mod callop;
mod callvalue;
mod chainid;
mod codecopy;
mod codesize;
mod comparator;
mod create;
mod dummy;
mod dup;
mod end_block;
mod end_chunk;
mod end_tx;
mod error_code_store;
mod error_invalid_creation_code;
mod error_invalid_jump;
mod error_invalid_opcode;
mod error_oog_account_access;
mod error_oog_call;
mod error_oog_constant;
mod error_oog_create;
mod error_oog_dynamic_memory;
mod error_oog_exp;
mod error_oog_log;
mod error_oog_memory_copy;
mod error_oog_precompile;
mod error_oog_sha3;
mod error_oog_sload_sstore;
mod error_oog_static_memory;
mod error_precompile_failed;
mod error_return_data_oo_bound;
mod error_stack;
mod error_write_protection;
mod exp;
mod extcodecopy;
mod extcodehash;
mod extcodesize;
mod gas;
mod gasprice;
mod invalid_tx;
mod is_zero;
mod jump;
mod jumpdest;
mod jumpi;
mod logs;
mod memory;
mod msize;
mod mul_div_mod;
mod mulmod;
#[path = "execution/not.rs"]
mod opcode_not;
mod origin;
mod padding;
mod pc;
mod pop;
mod precompiles;
mod push;
mod return_revert;
mod returndatacopy;
mod returndatasize;
mod sar;
mod sdiv_smod;
mod selfbalance;
mod sha3;
mod shl_shr;
mod signed_comparator;
mod signextend;
mod sload;
mod sstore;
mod stop;
mod swap;
mod tload;
mod tstore;

use self::{
    begin_chunk::BeginChunkGadget, block_ctx::BlockCtxGadget, end_chunk::EndChunkGadget,
    sha3::Sha3Gadget,
};
use add_sub::AddSubGadget;
use addmod::AddModGadget;
use address::AddressGadget;
use balance::BalanceGadget;
use begin_tx::BeginTxGadget;
use bitwise::BitwiseGadget;
use blockhash::BlockHashGadget;
use byte::ByteGadget;
use calldatacopy::CallDataCopyGadget;
use calldataload::CallDataLoadGadget;
use calldatasize::CallDataSizeGadget;
use caller::CallerGadget;
use callop::CallOpGadget;
use callvalue::CallValueGadget;
use chainid::ChainIdGadget;
use codecopy::CodeCopyGadget;
use codesize::CodesizeGadget;
use comparator::ComparatorGadget;
use create::CreateGadget;
use dummy::DummyGadget;
use dup::DupGadget;
use end_block::EndBlockGadget;
use end_tx::EndTxGadget;
use error_code_store::ErrorCodeStoreGadget;
use error_invalid_creation_code::ErrorInvalidCreationCodeGadget;
use error_invalid_jump::ErrorInvalidJumpGadget;
use error_invalid_opcode::ErrorInvalidOpcodeGadget;
use error_oog_account_access::ErrorOOGAccountAccessGadget;
use error_oog_call::ErrorOOGCallGadget;
use error_oog_constant::ErrorOOGConstantGadget;
use error_oog_create::ErrorOOGCreateGadget;
use error_oog_dynamic_memory::ErrorOOGDynamicMemoryGadget;
use error_oog_exp::ErrorOOGExpGadget;
use error_oog_log::ErrorOOGLogGadget;
use error_oog_memory_copy::ErrorOOGMemoryCopyGadget;
use error_oog_sha3::ErrorOOGSha3Gadget;
use error_oog_sload_sstore::ErrorOOGSloadSstoreGadget;
use error_oog_static_memory::ErrorOOGStaticMemoryGadget;
use error_precompile_failed::ErrorPrecompileFailedGadget;
use error_return_data_oo_bound::ErrorReturnDataOutOfBoundGadget;
use error_stack::ErrorStackGadget;
use error_write_protection::ErrorWriteProtectionGadget;
use exp::ExponentiationGadget;
use extcodecopy::ExtcodecopyGadget;
use extcodehash::ExtcodehashGadget;
use extcodesize::ExtcodesizeGadget;
use gas::GasGadget;
use gasprice::GasPriceGadget;
use invalid_tx::InvalidTxGadget;
use is_zero::IsZeroGadget;
use jump::JumpGadget;
use jumpdest::JumpdestGadget;
use jumpi::JumpiGadget;
use logs::LogGadget;

use crate::evm_circuit::execution::error_oog_precompile::ErrorOOGPrecompileGadget;
use memory::MemoryGadget;
use msize::MsizeGadget;
use mul_div_mod::MulDivModGadget;
use mulmod::MulModGadget;
use opcode_not::NotGadget;
use origin::OriginGadget;
use padding::PaddingGadget;
use pc::PcGadget;
use pop::PopGadget;
use precompiles::{EcrecoverGadget, IdentityGadget};
use push::PushGadget;
use return_revert::ReturnRevertGadget;
use returndatacopy::ReturnDataCopyGadget;
use returndatasize::ReturnDataSizeGadget;
use sar::SarGadget;
use sdiv_smod::SignedDivModGadget;
use selfbalance::SelfbalanceGadget;
use shl_shr::ShlShrGadget;
use signed_comparator::SignedComparatorGadget;
use signextend::SignextendGadget;
use sload::SloadGadget;
use sstore::SstoreGadget;
use stop::StopGadget;
use swap::SwapGadget;
use tload::TloadGadget;
use tstore::TstoreGadget;

pub(crate) trait ExecutionGadget<F: Field> {
    const NAME: &'static str;

    const EXECUTION_STATE: ExecutionState;

    fn configure(cb: &mut EVMConstraintBuilder<F>) -> Self;

    #[allow(clippy::too_many_arguments)]
    fn assign_exec_step(
        &self,
        region: &mut CachedRegion<'_, '_, F>,
        offset: usize,
        block: &Block<F>,
        chunk: &Chunk<F>,
        transaction: &Transaction,
        call: &Call,
        step: &ExecStep,
    ) -> Result<(), Error>;
}

#[derive(Clone, Debug)]
pub struct ExecutionConfig<F> {
    // EVM Circuit selector, which enables all usable rows.  The rows where this selector is
    // disabled won't verify any constraint (they can be unused rows or rows with blinding
    // factors).
    q_usable: Selector,
    // Dynamic selector that is enabled at the rows where each assigned execution step starts (a
    // step has dynamic height).
    q_step: Column<Advice>,
    // Column to hold constant values used for copy constraints
    constants: Column<Fixed>,
    num_rows_until_next_step: Column<Advice>,
    num_rows_inv: Column<Advice>,
    // Selector enabled in the row where the first execution step starts.
    q_step_first: Selector,
    // Selector enabled in the row where the last execution step starts.
    q_step_last: Selector,
    advices: [Column<Advice>; STEP_WIDTH],
    step: Step<F>,
    pub(crate) height_map: HashMap<ExecutionState, usize>,
    stored_expressions_map: HashMap<ExecutionState, Vec<StoredExpression<F>>>,
    debug_expressions_map: HashMap<ExecutionState, Vec<(String, Expression<F>)>>,
    instrument: Instrument,
    // internal state gadgets
    begin_tx_gadget: Box<BeginTxGadget<F>>,
    end_block_gadget: Box<EndBlockGadget<F>>,
    padding_gadget: Box<PaddingGadget<F>>,
    end_tx_gadget: Box<EndTxGadget<F>>,
    begin_chunk_gadget: Box<BeginChunkGadget<F>>,
    end_chunk_gadget: Box<EndChunkGadget<F>>,
    // opcode gadgets
    add_sub_gadget: Box<AddSubGadget<F>>,
    addmod_gadget: Box<AddModGadget<F>>,
    address_gadget: Box<AddressGadget<F>>,
    balance_gadget: Box<BalanceGadget<F>>,
    bitwise_gadget: Box<BitwiseGadget<F>>,
    byte_gadget: Box<ByteGadget<F>>,
    call_op_gadget: Box<CallOpGadget<F>>,
    call_value_gadget: Box<CallValueGadget<F>>,
    calldatacopy_gadget: Box<CallDataCopyGadget<F>>,
    calldataload_gadget: Box<CallDataLoadGadget<F>>,
    calldatasize_gadget: Box<CallDataSizeGadget<F>>,
    caller_gadget: Box<CallerGadget<F>>,
    chainid_gadget: Box<ChainIdGadget<F>>,
    codecopy_gadget: Box<CodeCopyGadget<F>>,
    codesize_gadget: Box<CodesizeGadget<F>>,
    comparator_gadget: Box<ComparatorGadget<F>>,
    dup_gadget: Box<DupGadget<F>>,
    exp_gadget: Box<ExponentiationGadget<F>>,
    extcodehash_gadget: Box<ExtcodehashGadget<F>>,
    extcodesize_gadget: Box<ExtcodesizeGadget<F>>,
    extcodecopy_gadget: Box<ExtcodecopyGadget<F>>,
    gas_gadget: Box<GasGadget<F>>,
    gasprice_gadget: Box<GasPriceGadget<F>>,
    iszero_gadget: Box<IsZeroGadget<F>>,
    jump_gadget: Box<JumpGadget<F>>,
    jumpdest_gadget: Box<JumpdestGadget<F>>,
    jumpi_gadget: Box<JumpiGadget<F>>,
    log_gadget: Box<LogGadget<F>>,
    memory_gadget: Box<MemoryGadget<F>>,
    msize_gadget: Box<MsizeGadget<F>>,
    mul_div_mod_gadget: Box<MulDivModGadget<F>>,
    mulmod_gadget: Box<MulModGadget<F>>,
    not_gadget: Box<NotGadget<F>>,
    origin_gadget: Box<OriginGadget<F>>,
    pc_gadget: Box<PcGadget<F>>,
    pop_gadget: Box<PopGadget<F>>,
    push_gadget: Box<PushGadget<F>>,
    return_revert_gadget: Box<ReturnRevertGadget<F>>,
    sar_gadget: Box<SarGadget<F>>,
    sdiv_smod_gadget: Box<SignedDivModGadget<F>>,
    selfbalance_gadget: Box<SelfbalanceGadget<F>>,
    sha3_gadget: Box<Sha3Gadget<F>>,
    shl_shr_gadget: Box<ShlShrGadget<F>>,
    returndatasize_gadget: Box<ReturnDataSizeGadget<F>>,
    returndatacopy_gadget: Box<ReturnDataCopyGadget<F>>,
    create_gadget: Box<CreateGadget<F, false, { ExecutionState::CREATE }>>,
    create2_gadget: Box<CreateGadget<F, true, { ExecutionState::CREATE2 }>>,
    selfdestruct_gadget: Box<DummyGadget<F, 1, 0, { ExecutionState::SELFDESTRUCT }>>,
    signed_comparator_gadget: Box<SignedComparatorGadget<F>>,
    signextend_gadget: Box<SignextendGadget<F>>,
    sload_gadget: Box<SloadGadget<F>>,
    sstore_gadget: Box<SstoreGadget<F>>,
    tload_gadget: Box<TloadGadget<F>>,
    tstore_gadget: Box<TstoreGadget<F>>,
    stop_gadget: Box<StopGadget<F>>,
    swap_gadget: Box<SwapGadget<F>>,
    blockhash_gadget: Box<BlockHashGadget<F>>,
    block_ctx_gadget: Box<BlockCtxGadget<F>>,
    // error gadgets
    error_oog_call: Box<ErrorOOGCallGadget<F>>,
    error_oog_precompile: Box<ErrorOOGPrecompileGadget<F>>,
    error_oog_constant: Box<ErrorOOGConstantGadget<F>>,
    error_oog_exp: Box<ErrorOOGExpGadget<F>>,
    error_oog_memory_copy: Box<ErrorOOGMemoryCopyGadget<F>>,
    error_oog_sload_sstore: Box<ErrorOOGSloadSstoreGadget<F>>,
    error_oog_static_memory_gadget: Box<ErrorOOGStaticMemoryGadget<F>>,
    error_stack: Box<ErrorStackGadget<F>>,
    error_write_protection: Box<ErrorWriteProtectionGadget<F>>,
    error_oog_dynamic_memory_gadget: Box<ErrorOOGDynamicMemoryGadget<F>>,
    error_oog_log: Box<ErrorOOGLogGadget<F>>,
    error_oog_sha3: Box<ErrorOOGSha3Gadget<F>>,
    error_oog_account_access: Box<ErrorOOGAccountAccessGadget<F>>,
    error_oog_ext_codecopy: Box<DummyGadget<F, 0, 0, { ExecutionState::ErrorOutOfGasEXTCODECOPY }>>,
    error_oog_create: Box<ErrorOOGCreateGadget<F>>,
    error_oog_self_destruct:
        Box<DummyGadget<F, 0, 0, { ExecutionState::ErrorOutOfGasSELFDESTRUCT }>>,
    error_oog_code_store: Box<ErrorCodeStoreGadget<F>>,
    error_invalid_jump: Box<ErrorInvalidJumpGadget<F>>,
    error_invalid_opcode: Box<ErrorInvalidOpcodeGadget<F>>,
    #[allow(dead_code, reason = "under active development")]
    error_depth: Box<DummyGadget<F, 0, 0, { ExecutionState::ErrorDepth }>>,
    #[allow(dead_code, reason = "under active development")]
    error_contract_address_collision:
        Box<DummyGadget<F, 0, 0, { ExecutionState::ErrorContractAddressCollision }>>,
    error_invalid_creation_code: Box<ErrorInvalidCreationCodeGadget<F>>,
    error_precompile_failed: Box<ErrorPrecompileFailedGadget<F>>,
    error_return_data_out_of_bound: Box<ErrorReturnDataOutOfBoundGadget<F>>,
    // precompile calls
    precompile_ecrecover_gadget: Box<EcrecoverGadget<F>>,
    precompile_identity_gadget: Box<IdentityGadget<F>>,
    invalid_tx: Option<Box<InvalidTxGadget<F>>>,
}

type TxCallStep<'a> = (&'a Transaction, &'a Call, &'a ExecStep);

impl<F: Field> ExecutionConfig<F> {
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::redundant_closure_call)]
    pub(crate) fn configure(
        meta: &mut ConstraintSystem<F>,
        challenges: Challenges<Expression<F>>,
        fixed_table: &dyn LookupTable<F>,
        u8_table: &dyn LookupTable<F>,
        u16_table: &dyn LookupTable<F>,
        tx_table: &dyn LookupTable<F>,
        rw_table: &dyn LookupTable<F>,
        bytecode_table: &dyn LookupTable<F>,
        block_table: &dyn LookupTable<F>,
        copy_table: &dyn LookupTable<F>,
        keccak_table: &dyn LookupTable<F>,
        exp_table: &dyn LookupTable<F>,
        sig_table: &dyn LookupTable<F>,
        chunk_ctx_table: &dyn LookupTable<F>,
        is_first_chunk: &IsZeroConfig<F>,
        is_last_chunk: &IsZeroConfig<F>,
        feature_config: FeatureConfig,
    ) -> Self {
        let mut instrument = Instrument::default();
        let q_usable = meta.complex_selector();
        let q_step = meta.advice_column();
        let constants = meta.fixed_column();
        meta.enable_constant(constants);
        let num_rows_until_next_step = meta.advice_column();
        let num_rows_inv = meta.advice_column();
        let q_step_first = meta.complex_selector();
        let q_step_last = meta.complex_selector();

        let advices = [(); STEP_WIDTH]
            .iter()
            .enumerate()
            .map(|(n, _)| {
                if n < EVM_LOOKUP_COLS {
                    meta.advice_column_in(ThirdPhase)
                } else if n < EVM_LOOKUP_COLS + N_PHASE2_COLUMNS {
                    meta.advice_column_in(SecondPhase)
                } else {
                    meta.advice_column_in(FirstPhase)
                }
            })
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        let step_curr = Step::new(meta, advices, 0);
        let mut height_map = HashMap::new();
        let (execute_state_first_step_whitelist, execute_state_last_step_whitelist) = (
            HashSet::from_iter(
                vec![
                    ExecutionState::BeginTx,
                    ExecutionState::Padding,
                    ExecutionState::BeginChunk,
                ]
                .into_iter()
                .chain(
                    feature_config
                        .invalid_tx
                        .then_some(ExecutionState::InvalidTx),
                ),
            ),
            HashSet::from([ExecutionState::EndBlock, ExecutionState::EndChunk]),
        );

        meta.create_gate("Constrain execution state", |meta| {
            let q_usable = meta.query_selector(q_usable);
            let q_step = meta.query_advice(q_step, Rotation::cur());
            let q_step_first = meta.query_selector(q_step_first);
            let q_step_last = meta.query_selector(q_step_last);

            let execution_state_selector_constraints = step_curr.state.execution_state.configure();

            let first_step_first_chunk_check = {
                let exestates = step_curr
                    .execution_state_selector(execute_state_first_step_whitelist.iter().cloned());
                iter::once((
                    "First step first chunk should be BeginTx or EndBlock or BeginChunk",
                    (1.expr() - is_first_chunk.expr())
                        * q_step_first.clone()
                        * (1.expr() - exestates),
                ))
            };

            let first_step_non_first_chunk_check = {
                let begin_chunk_selector =
                    step_curr.execution_state_selector([ExecutionState::BeginChunk]);
                iter::once((
                    "First step (non first chunk) should be BeginChunk",
                    (1.expr() - is_first_chunk.expr())
                        * q_step_first
                        * (1.expr() - begin_chunk_selector),
                ))
            };

            let last_step_last_chunk_check = {
                let end_block_selector =
                    step_curr.execution_state_selector([ExecutionState::EndBlock]);
                iter::once((
                    "Last step last chunk should be EndBlock",
                    is_last_chunk.expr() * q_step_last.clone() * (1.expr() - end_block_selector),
                ))
            };

            let last_step_non_last_chunk_check = {
                let end_chunk_selector =
                    step_curr.execution_state_selector([ExecutionState::EndChunk]);
                iter::once((
                    "Last step (non last chunk) should be EndChunk",
                    (1.expr() - is_last_chunk.expr())
                        * q_step_last
                        * (1.expr() - end_chunk_selector),
                ))
            };

            execution_state_selector_constraints
                .into_iter()
                .map(move |(name, poly)| (name, q_usable.clone() * q_step.clone() * poly))
                .chain(first_step_first_chunk_check)
                .chain(first_step_non_first_chunk_check)
                .chain(last_step_last_chunk_check)
                .chain(last_step_non_last_chunk_check)
        });

        meta.create_gate("q_step", |meta| {
            let q_usable = meta.query_selector(q_usable);
            let q_step_first = meta.query_selector(q_step_first);
            let q_step_last = meta.query_selector(q_step_last);
            let q_step = meta.query_advice(q_step, Rotation::cur());
            let num_rows_left_cur = meta.query_advice(num_rows_until_next_step, Rotation::cur());
            let num_rows_left_next = meta.query_advice(num_rows_until_next_step, Rotation::next());
            let num_rows_left_inverse = meta.query_advice(num_rows_inv, Rotation::cur());

            let mut cb = BaseConstraintBuilder::default();
            // q_step needs to be enabled on the first row
            // rw_counter starts at 1
            cb.condition(q_step_first, |cb| {
                cb.require_equal("q_step == 1", q_step.clone(), 1.expr());
                cb.require_equal(
                    "inner_rw_counter is initialized to be 1",
                    step_curr.state.inner_rw_counter.expr(),
                    1.expr(),
                )
            });
            // For every step, is_create and is_root are boolean.
            cb.condition(q_step.clone(), |cb| {
                cb.require_boolean(
                    "step.is_create is boolean",
                    step_curr.state.is_create.expr(),
                );
                cb.require_boolean("step.is_root is boolean", step_curr.state.is_root.expr());
            });
            // q_step needs to be enabled on the last row
            cb.condition(q_step_last, |cb| {
                cb.require_equal("q_step == 1", q_step.clone(), 1.expr());
            });
            // Except when step is enabled, the step counter needs to decrease by 1
            cb.condition(1.expr() - q_step.clone(), |cb| {
                cb.require_equal(
                    "num_rows_left_cur := num_rows_left_next + 1",
                    num_rows_left_cur.clone(),
                    num_rows_left_next + 1.expr(),
                );
            });
            // Enforce that q_step := num_rows_until_next_step == 0
            let is_zero = 1.expr() - (num_rows_left_cur.clone() * num_rows_left_inverse.clone());
            cb.require_zero(
                "num_rows_left_cur * is_zero == 0",
                num_rows_left_cur * is_zero.clone(),
            );
            cb.require_zero(
                "num_rows_left_inverse * is_zero == 0",
                num_rows_left_inverse * is_zero.clone(),
            );
            cb.require_equal("q_step == is_zero", q_step, is_zero);
            // On each usable row
            cb.gate(q_usable)
        });

        let mut stored_expressions_map = HashMap::new();
        let mut debug_expressions_map = HashMap::new();

        macro_rules! configure_gadget {
            () => {
                // We create each gadget in a closure so that the stack required to hold
                // the gadget value before being copied to the box is freed immediately after
                // the boxed gadget is returned.
                // We put each gadget in a box so that they stay in the heap to keep
                // ExecutionConfig at a manageable size.
                (|| {
                    Box::new(Self::configure_gadget(
                        meta,
                        advices,
                        &challenges,
                        q_usable,
                        q_step,
                        num_rows_until_next_step,
                        q_step_first,
                        q_step_last,
                        &step_curr,
                        chunk_ctx_table,
                        &execute_state_first_step_whitelist,
                        &execute_state_last_step_whitelist,
                        &mut height_map,
                        &mut stored_expressions_map,
                        &mut debug_expressions_map,
                        &mut instrument,
                        feature_config.clone(),
                    ))
                })()
            };
        }

        let cell_manager = step_curr.cell_manager.clone();

        let config = Self {
            q_usable,
            q_step,
            constants,
            num_rows_until_next_step,
            num_rows_inv,
            q_step_first,
            q_step_last,
            advices,
            // internal states
            begin_tx_gadget: configure_gadget!(),
            padding_gadget: configure_gadget!(),
            end_tx_gadget: configure_gadget!(),
            begin_chunk_gadget: configure_gadget!(),
            end_chunk_gadget: configure_gadget!(),
            end_block_gadget: configure_gadget!(),
            invalid_tx: feature_config.invalid_tx.then(|| configure_gadget!()),
            // opcode gadgets
            add_sub_gadget: configure_gadget!(),
            addmod_gadget: configure_gadget!(),
            bitwise_gadget: configure_gadget!(),
            byte_gadget: configure_gadget!(),
            call_op_gadget: configure_gadget!(),
            call_value_gadget: configure_gadget!(),
            calldatacopy_gadget: configure_gadget!(),
            calldataload_gadget: configure_gadget!(),
            calldatasize_gadget: configure_gadget!(),
            caller_gadget: configure_gadget!(),
            chainid_gadget: configure_gadget!(),
            codecopy_gadget: configure_gadget!(),
            codesize_gadget: configure_gadget!(),
            comparator_gadget: configure_gadget!(),
            dup_gadget: configure_gadget!(),
            extcodehash_gadget: configure_gadget!(),
            extcodesize_gadget: configure_gadget!(),
            gas_gadget: configure_gadget!(),
            gasprice_gadget: configure_gadget!(),
            iszero_gadget: configure_gadget!(),
            jump_gadget: configure_gadget!(),
            jumpdest_gadget: configure_gadget!(),
            jumpi_gadget: configure_gadget!(),
            log_gadget: configure_gadget!(),
            memory_gadget: configure_gadget!(),
            msize_gadget: configure_gadget!(),
            mul_div_mod_gadget: configure_gadget!(),
            mulmod_gadget: configure_gadget!(),
            not_gadget: configure_gadget!(),
            origin_gadget: configure_gadget!(),
            pc_gadget: configure_gadget!(),
            pop_gadget: configure_gadget!(),
            push_gadget: configure_gadget!(),
            return_revert_gadget: configure_gadget!(),
            sdiv_smod_gadget: configure_gadget!(),
            selfbalance_gadget: configure_gadget!(),
            sha3_gadget: configure_gadget!(),
            address_gadget: configure_gadget!(),
            balance_gadget: configure_gadget!(),
            blockhash_gadget: configure_gadget!(),
            exp_gadget: configure_gadget!(),
            sar_gadget: configure_gadget!(),
            extcodecopy_gadget: configure_gadget!(),
            returndatasize_gadget: configure_gadget!(),
            returndatacopy_gadget: configure_gadget!(),
            create_gadget: configure_gadget!(),
            create2_gadget: configure_gadget!(),
            selfdestruct_gadget: configure_gadget!(),
            shl_shr_gadget: configure_gadget!(),
            signed_comparator_gadget: configure_gadget!(),
            signextend_gadget: configure_gadget!(),
            sload_gadget: configure_gadget!(),
            sstore_gadget: configure_gadget!(),
            tload_gadget: configure_gadget!(),
            tstore_gadget: configure_gadget!(),
            stop_gadget: configure_gadget!(),
            swap_gadget: configure_gadget!(),
            block_ctx_gadget: configure_gadget!(),
            // error gadgets
            error_oog_constant: configure_gadget!(),
            error_oog_static_memory_gadget: configure_gadget!(),
            error_stack: configure_gadget!(),
            error_oog_dynamic_memory_gadget: configure_gadget!(),
            error_oog_log: configure_gadget!(),
            error_oog_sload_sstore: configure_gadget!(),
            error_oog_call: configure_gadget!(),
            error_oog_precompile: configure_gadget!(),
            error_oog_memory_copy: configure_gadget!(),
            error_oog_account_access: configure_gadget!(),
            error_oog_sha3: configure_gadget!(),
            error_oog_ext_codecopy: configure_gadget!(),
            error_oog_exp: configure_gadget!(),
            error_oog_create: configure_gadget!(),
            error_oog_self_destruct: configure_gadget!(),
            error_oog_code_store: configure_gadget!(),
            error_invalid_jump: configure_gadget!(),
            error_invalid_opcode: configure_gadget!(),
            error_write_protection: configure_gadget!(),
            error_depth: configure_gadget!(),
            error_contract_address_collision: configure_gadget!(),
            error_invalid_creation_code: configure_gadget!(),
            error_precompile_failed: configure_gadget!(),
            error_return_data_out_of_bound: configure_gadget!(),
            // precompile calls
            precompile_identity_gadget: configure_gadget!(),
            precompile_ecrecover_gadget: configure_gadget!(),
            // step and presets
            step: step_curr,
            height_map,
            stored_expressions_map,
            debug_expressions_map,
            instrument,
        };

        Self::configure_lookup(
            meta,
            fixed_table,
            u8_table,
            u16_table,
            tx_table,
            rw_table,
            bytecode_table,
            block_table,
            copy_table,
            keccak_table,
            exp_table,
            sig_table,
            chunk_ctx_table,
            &challenges,
            &cell_manager,
        );
        config
    }

    pub fn instrument(&self) -> &Instrument {
        &self.instrument
    }

    #[allow(clippy::too_many_arguments)]
    fn configure_gadget<G: ExecutionGadget<F>>(
        meta: &mut ConstraintSystem<F>,
        advices: [Column<Advice>; STEP_WIDTH],
        challenges: &Challenges<Expression<F>>,
        q_usable: Selector,
        q_step: Column<Advice>,
        num_rows_until_next_step: Column<Advice>,
        q_step_first: Selector,
        q_step_last: Selector,
        step_curr: &Step<F>,
        chunk_ctx_table: &dyn LookupTable<F>,
        execute_state_first_step_whitelist: &HashSet<ExecutionState>,
        execute_state_last_step_whitelist: &HashSet<ExecutionState>,
        height_map: &mut HashMap<ExecutionState, usize>,
        stored_expressions_map: &mut HashMap<ExecutionState, Vec<StoredExpression<F>>>,
        debug_expressions_map: &mut HashMap<ExecutionState, Vec<(String, Expression<F>)>>,
        instrument: &mut Instrument,
        feature_config: FeatureConfig,
    ) -> G {
        // Configure the gadget with the max height first so we can find out the actual
        // height
        let height = {
            let dummy_step_next = Step::new(meta, advices, MAX_STEP_HEIGHT);
            let mut cb = EVMConstraintBuilder::new(
                meta,
                step_curr.clone(),
                dummy_step_next,
                challenges,
                G::EXECUTION_STATE,
                feature_config,
            );
            G::configure(&mut cb);
            let (_, _, height, _) = cb.build();
            height
        };

        // Now actually configure the gadget with the correct minimal height
        let step_next = &Step::new(meta, advices, height);

        let mut cb = EVMConstraintBuilder::new(
            meta,
            step_curr.clone(),
            step_next.clone(),
            challenges,
            G::EXECUTION_STATE,
            feature_config,
        );

        let gadget = G::configure(&mut cb);

        Self::configure_gadget_impl(
            q_usable,
            q_step,
            num_rows_until_next_step,
            q_step_first,
            q_step_last,
            step_curr,
            step_next,
            height_map,
            stored_expressions_map,
            debug_expressions_map,
            execute_state_first_step_whitelist,
            execute_state_last_step_whitelist,
            instrument,
            G::NAME,
            G::EXECUTION_STATE,
            height,
            cb,
            chunk_ctx_table,
            challenges,
        );

        gadget
    }

    #[allow(clippy::too_many_arguments)]
    fn configure_gadget_impl(
        q_usable: Selector,
        q_step: Column<Advice>,
        num_rows_until_next_step: Column<Advice>,
        q_step_first: Selector,
        q_step_last: Selector,
        step_curr: &Step<F>,
        step_next: &Step<F>,
        height_map: &mut HashMap<ExecutionState, usize>,
        stored_expressions_map: &mut HashMap<ExecutionState, Vec<StoredExpression<F>>>,
        debug_expressions_map: &mut HashMap<ExecutionState, Vec<(String, Expression<F>)>>,
        execute_state_first_step_whitelist: &HashSet<ExecutionState>,
        execute_state_last_step_whitelist: &HashSet<ExecutionState>,
        instrument: &mut Instrument,
        name: &'static str,
        execution_state: ExecutionState,
        height: usize,
        mut cb: EVMConstraintBuilder<F>,
        chunk_ctx_table: &dyn LookupTable<F>,
        challenges: &Challenges<Expression<F>>,
    ) {
        // Enforce the step height for this opcode
        let num_rows_until_next_step_next = cb
            .query_expression(|meta| meta.query_advice(num_rows_until_next_step, Rotation::next()));
        cb.require_equal(
            "num_rows_until_next_step_next := height - 1",
            num_rows_until_next_step_next,
            (height - 1).expr(),
        );

        instrument.on_gadget_built(execution_state, &cb);

        let step_curr_rw_counter = cb.curr.state.rw_counter.clone();
        let step_curr_rw_counter_offset = cb.rw_counter_offset();
        if execution_state == ExecutionState::BeginChunk {
            cb.debug_expression("step_curr_rw_counter.expr()", step_curr_rw_counter.expr());
        }

        let debug_expressions = cb.debug_expressions.clone();

        // Extract feature config here before cb is built.
        let enable_invalid_tx = cb.feature_config.invalid_tx;

        let (constraints, stored_expressions, _, meta) = cb.build();
        debug_assert!(
            !height_map.contains_key(&execution_state),
            "execution state already configured"
        );

        height_map.insert(execution_state, height);
        debug_assert!(
            !stored_expressions_map.contains_key(&execution_state),
            "execution state already configured"
        );
        stored_expressions_map.insert(execution_state, stored_expressions);
        debug_expressions_map.insert(execution_state, debug_expressions);

        // Enforce the logic for this opcode
        let sel_step: &dyn Fn(&mut VirtualCells<F>) -> Expression<F> =
            &|meta| meta.query_advice(q_step, Rotation::cur());
        let sel_step_first: &dyn Fn(&mut VirtualCells<F>) -> Expression<F> =
            &|meta| meta.query_selector(q_step_first);
        let sel_step_last: &dyn Fn(&mut VirtualCells<F>) -> Expression<F> =
            &|meta| meta.query_selector(q_step_last);
        let sel_not_step_last: &dyn Fn(&mut VirtualCells<F>) -> Expression<F> = &|meta| {
            meta.query_advice(q_step, Rotation::cur()) * not::expr(meta.query_selector(q_step_last))
        };

        for (selector, constraints) in [
            (sel_step, constraints.step),
            (sel_step_first, constraints.step_first),
            (sel_step_last, constraints.step_last),
            (sel_not_step_last, constraints.not_step_last),
        ] {
            if !constraints.is_empty() {
                meta.create_gate(name, |meta| {
                    let q_usable = meta.query_selector(q_usable);
                    let selector = selector(meta);
                    constraints.into_iter().map(move |(name, constraint)| {
                        (name, q_usable.clone() * selector.clone() * constraint)
                    })
                });
            }
        }

        // constraint global rw counter value at first/last step via chunk_ctx_table lookup
        // we can't do it inside constraint_builder(cb)
        // because lookup expression in constraint builder DO NOT support apply conditional
        // `step_first/step_last` selector at lookup cell.
        if execute_state_first_step_whitelist.contains(&execution_state) {
            meta.lookup_any("first must lookup initial rw_counter", |meta| {
                let q_usable = meta.query_selector(q_usable);
                let q_step_first = meta.query_selector(q_step_first);
                let execute_state_selector = step_curr.execution_state_selector([execution_state]);

                vec![(
                    q_usable
                        * q_step_first
                        * execute_state_selector
                        * rlc::expr(
                            &[
                                ChunkCtxFieldTag::InitialRWC.expr(),
                                step_curr.state.rw_counter.expr(),
                            ],
                            challenges.lookup_input(),
                        ),
                    rlc::expr(
                        &chunk_ctx_table.table_exprs(meta),
                        challenges.lookup_input(),
                    ),
                )]
            });
        }

        if execute_state_last_step_whitelist.contains(&execution_state) {
            meta.lookup_any("last step must lookup end rw_counter", |meta| {
                let q_usable = meta.query_selector(q_usable);
                let q_step_last = meta.query_selector(q_step_last);
                let execute_state_selector = step_curr.execution_state_selector([execution_state]);
                vec![(
                    q_usable
                        * q_step_last
                        * execute_state_selector
                        * rlc::expr(
                            &[
                                ChunkCtxFieldTag::EndRWC.expr(),
                                step_curr_rw_counter.expr() + step_curr_rw_counter_offset.clone(),
                            ],
                            challenges.lookup_input(),
                        ),
                    rlc::expr(
                        &chunk_ctx_table.table_exprs(meta),
                        challenges.lookup_input(),
                    ),
                )]
            });
        }

        // Enforce the state transitions for this opcode
        meta.create_gate("Constrain state machine transitions", |meta| {
            let q_usable = meta.query_selector(q_usable);
            let q_step = meta.query_advice(q_step, Rotation::cur());
            let q_step_last = meta.query_selector(q_step_last);

            // ExecutionState transition should be correct.
            iter::empty()
                .chain(
                    [
                        (
                            "EndTx can only transit to BeginTx or Padding or EndBlock or EndChunk or InvalidTx",
                            ExecutionState::EndTx,
                            vec![
                                ExecutionState::BeginTx,
                                ExecutionState::EndBlock,
                                ExecutionState::Padding,
                                ExecutionState::EndChunk,
                            ].into_iter()
                            .chain(enable_invalid_tx.then_some(ExecutionState::InvalidTx))
                            .collect(),
                        ),
                        (
                            "EndChunk can only transit to EndChunk",
                            ExecutionState::EndChunk,
                            vec![ExecutionState::EndChunk],
                        ),
                        (
                            "Padding can only transit to Padding or EndBlock or EndChunk",
                            ExecutionState::Padding,
                            vec![
                                ExecutionState::Padding,
                                ExecutionState::EndBlock,
                                ExecutionState::EndChunk,
                            ],
                        ),
                        (
                            "EndBlock can only transit to EndBlock",
                            ExecutionState::EndBlock,
                            vec![ExecutionState::EndBlock],
                        ),
                    ]
                    .into_iter()
                    .filter(move |(_, from, _)| *from == execution_state)
                    .map(|(_, _, to)| 1.expr() - step_next.execution_state_selector(to)),
                )
                .chain(
                    [
                        (
                            "Only EndTx and InvalidTx can transit to BeginTx",
                            ExecutionState::BeginTx,
                            iter::once(ExecutionState::EndTx)
                                .chain(enable_invalid_tx.then_some(ExecutionState::InvalidTx))
                                .collect(),
                        ),
                        (
                            "Only ExecutionState which halts or BeginTx can transit to EndTx",
                            ExecutionState::EndTx,
                            ExecutionState::iter()
                                .filter(ExecutionState::halts)
                                .chain(iter::once(ExecutionState::BeginTx))
                                .collect(),
                        ),
                        (
                            "Only BeginChunk or EndTx or InvalidTx or EndBlock or Padding can transit to EndBlock",
                            ExecutionState::EndBlock,
                            vec![
                                ExecutionState::BeginChunk,
                                ExecutionState::EndTx,
                                ExecutionState::EndBlock,
                                ExecutionState::Padding,
                            ].into_iter()
                            .chain(enable_invalid_tx.then_some(ExecutionState::InvalidTx))
                            .collect(),
                        ),
                        (
                            "Only BeginChunk can transit to BeginChunk",
                            ExecutionState::BeginChunk,
                            vec![ExecutionState::BeginChunk],
                        ),
                    ]
                    .into_iter()
                    .chain(enable_invalid_tx.then(|| {
                        (
                            "Only EndTx and InvalidTx can transit to InvalidTx",
                            ExecutionState::InvalidTx,
                            vec![ExecutionState::EndTx, ExecutionState::InvalidTx],
                        )
                    }))
                    .filter(move |(_, _, from)| !from.contains(&execution_state))
                    .map(|(_, to, _)| step_next.execution_state_selector([to])),
                )
                // Accumulate all state transition checks.
                // This can be done because all summed values are enforced to be boolean.
                .reduce(|accum, poly| accum + poly)
                .map(move |poly| {
                    q_usable.clone()
                        * q_step.clone()
                        * (1.expr() - q_step_last.clone())
                        * step_curr.execution_state_selector([execution_state])
                        * poly
                })
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn configure_lookup(
        meta: &mut ConstraintSystem<F>,
        fixed_table: &dyn LookupTable<F>,
        u8_table: &dyn LookupTable<F>,
        u16_table: &dyn LookupTable<F>,
        tx_table: &dyn LookupTable<F>,
        rw_table: &dyn LookupTable<F>,
        bytecode_table: &dyn LookupTable<F>,
        block_table: &dyn LookupTable<F>,
        copy_table: &dyn LookupTable<F>,
        keccak_table: &dyn LookupTable<F>,
        exp_table: &dyn LookupTable<F>,
        sig_table: &dyn LookupTable<F>,
        chunk_ctx_table: &dyn LookupTable<F>,
        challenges: &Challenges<Expression<F>>,
        cell_manager: &CellManager<CMFixedWidthStrategy>,
    ) {
        for column in cell_manager.columns().iter() {
            if let CellType::Lookup(table) = column.cell_type {
                let name = format!("{:?}", table);
                let column_expr = column.expr(meta);
                meta.lookup_any(Box::leak(name.into_boxed_str()), |meta| {
                    let table_expressions = match table {
                        Table::Fixed => fixed_table,
                        Table::U8 => u8_table,
                        Table::U16 => u16_table,
                        Table::Tx => tx_table,
                        Table::Rw => rw_table,
                        Table::Bytecode => bytecode_table,
                        Table::Block => block_table,
                        Table::Copy => copy_table,
                        Table::Keccak => keccak_table,
                        Table::Exp => exp_table,
                        Table::Sig => sig_table,
                        Table::ChunkCtx => chunk_ctx_table,
                    }
                    .table_exprs(meta);
                    vec![(
                        column_expr,
                        rlc::expr(&table_expressions, challenges.lookup_input()),
                    )]
                });
            }
        }
    }

    /// Assign columns related to step counter
    fn assign_q_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        height: usize,
    ) -> Result<(), Error> {
        // Name Advice columns
        for idx in 0..height {
            let offset = offset + idx;
            self.q_usable.enable(region, offset)?;
            region.assign_advice(
                || "step selector",
                self.q_step,
                offset,
                || Value::known(if idx == 0 { F::ONE } else { F::ZERO }),
            )?;
            let value = if idx == 0 {
                F::ZERO
            } else {
                F::from((height - idx) as u64)
            };
            region.assign_advice(
                || "step height",
                self.num_rows_until_next_step,
                offset,
                || Value::known(value),
            )?;
            region.assign_advice(
                || "step height inv",
                self.num_rows_inv,
                offset,
                || Value::known(value.invert().unwrap_or(F::ZERO)),
            )?;
        }
        Ok(())
    }

    /// Assign block
    /// When exact is enabled, assign exact steps in block without padding for
    /// unit test purpose
    pub fn assign_block(
        &self,
        layouter: &mut impl Layouter<F>,
        block: &Block<F>,
        chunk: &Chunk<F>,
        challenges: &Challenges<Value<F>>,
    ) -> Result<usize, Error> {
        // Track number of calls to `layouter.assign_region` as layouter assignment passes.
        let mut assign_pass = 0;
        layouter.assign_region(
            || "Execution step",
            |mut region| {
                let mut offset = 0;

                // Annotate the EVMCircuit columns within it's single region.
                self.annotate_circuit(&mut region);

                self.q_step_first.enable(&mut region, offset)?;

                let dummy_tx = Transaction::default();
                // chunk_txs is just a super set of execstep including both belong to this chunk and
                // outside of this chunk
                let chunk_txs: &[Transaction] = block
                    .txs
                    .get(chunk.chunk_context.initial_tx_index..chunk.chunk_context.end_tx_index)
                    .unwrap_or_default();

                // If it's the very first chunk in a block set last call & begin_chunk to default
                let prev_chunk_last_call = chunk.prev_last_call.clone().unwrap_or_default();
                let cur_chunk_last_call = chunk_txs
                    .last()
                    .map(|tx| tx.calls()[0].clone())
                    .unwrap_or_else(|| prev_chunk_last_call.clone());

                let padding = chunk.padding.as_ref().expect("padding can't be None");

                // conditionally adding first step as begin chunk
                let maybe_begin_chunk = {
                    if let Some(begin_chunk) = &chunk.begin_chunk {
                        vec![(&dummy_tx, &prev_chunk_last_call, begin_chunk)]
                    } else {
                        vec![]
                    }
                };

                let mut tx_call_steps = maybe_begin_chunk
                    .into_iter()
                    .chain(chunk_txs.iter().flat_map(|tx| {
                        tx.steps()
                            .iter()
                            // chunk_txs is just a super set of execstep. To filter targeting
                            // execstep we need to further filter by [initial_rwc, end_rwc)
                            .filter(|step| {
                                step.rwc.0 >= chunk.chunk_context.initial_rwc
                                    && step.rwc.0 < chunk.chunk_context.end_rwc
                            })
                            .map(move |step| (tx, &tx.calls()[step.call_index], step))
                    }))
                    // this dummy step is just for real step assignment proceed to `second last`
                    .chain(std::iter::once((&dummy_tx, &cur_chunk_last_call, padding)))
                    .peekable();

                let evm_rows = chunk.fixed_param.max_evm_rows;

                let mut assign_padding_or_step = |cur_tx_call_step: TxCallStep,
                                                  mut offset: usize,
                                                  next_tx_call_step: Option<TxCallStep>,
                                                  padding_end: Option<usize>|
                 -> Result<usize, Error> {
                    let (_tx, call, step) = cur_tx_call_step;
                    let height = step.execution_state().get_step_height();

                    // If padding, assign padding range with (dummy_tx, call, step)
                    // otherwise, assign one row with cur (tx, call, step), with next (tx, call,
                    // step) to lookahead
                    if let Some(padding_end) = padding_end {
                        // padding_end is the absolute position over all rows,
                        // must be greater then current offset
                        if offset >= padding_end {
                            log::error!(
                                "evm circuit offset larger than padding: {} > {}",
                                offset,
                                padding_end
                            );
                            return Err(Error::Synthesis);
                        }
                        log::trace!("assign Padding in range [{},{})", offset, padding_end);
                        self.assign_same_exec_step_in_range(
                            &mut region,
                            offset,
                            padding_end,
                            block,
                            chunk,
                            (&dummy_tx, call, step),
                            height,
                            challenges,
                            assign_pass,
                        )?;
                        let padding_start = offset;
                        for row_idx in padding_start..padding_end {
                            self.assign_q_step(&mut region, row_idx, height)?;
                            offset += height;
                        }
                    } else {
                        self.assign_exec_step(
                            &mut region,
                            offset,
                            block,
                            chunk,
                            cur_tx_call_step,
                            height,
                            next_tx_call_step,
                            challenges,
                            assign_pass,
                        )?;
                        self.assign_q_step(&mut region, offset, height)?;
                        offset += height;
                    }

                    Ok(offset) // return latest offset
                };

                let mut second_last_real_step = None;
                let mut second_last_real_step_offset = 0;

                // part1: assign real steps
                while let Some(cur) = tx_call_steps.next() {
                    let next = tx_call_steps.peek();
                    if next.is_none() {
                        break;
                    }

                    second_last_real_step = Some(cur);
                    // record offset of current step before assignment
                    second_last_real_step_offset = offset;
                    offset = assign_padding_or_step(cur, offset, next.copied(), None)?;
                }

                // next step priority: padding > end_chunk > end_block
                let mut next_step_after_real_step = None;

                // part2: assign padding
                if evm_rows > 0 {
                    if next_step_after_real_step.is_none() {
                        next_step_after_real_step = Some(padding.clone());
                    }
                    offset = assign_padding_or_step(
                        (&dummy_tx, &cur_chunk_last_call, padding),
                        offset,
                        None,
                        Some(evm_rows - 1),
                    )?;
                }

                // part3: assign end chunk or end block
                if let Some(end_chunk) = &chunk.end_chunk {
                    debug_assert_eq!(ExecutionState::EndChunk.get_step_height(), 1);
                    offset = assign_padding_or_step(
                        (&dummy_tx, &cur_chunk_last_call, end_chunk),
                        offset,
                        None,
                        None,
                    )?;
                    if next_step_after_real_step.is_none() {
                        next_step_after_real_step = Some(end_chunk.clone());
                    }
                } else {
                    assert!(
                        chunk.chunk_context.is_last_chunk(),
                        "If not end_chunk, must be end_block at last chunk"
                    );
                    debug_assert_eq!(ExecutionState::EndBlock.get_step_height(), 1);
                    offset = assign_padding_or_step(
                        (&dummy_tx, &cur_chunk_last_call, &block.end_block),
                        offset,
                        None,
                        None,
                    )?;
                    if next_step_after_real_step.is_none() {
                        next_step_after_real_step = Some(block.end_block.clone());
                    }
                }

                // part4:
                // re-assigned real second last step, because we know next_step_after_real_step now
                assert!(next_step_after_real_step.is_some());
                if let Some(last_real_step) = second_last_real_step {
                    _ = assign_padding_or_step(
                        last_real_step,
                        second_last_real_step_offset,
                        Some((
                            &dummy_tx,
                            &cur_chunk_last_call,
                            &next_step_after_real_step.unwrap(),
                        )),
                        None,
                    )?;
                }

                // part5:
                // enable last row
                self.q_step_last.enable(&mut region, offset - 1)?; // offset - 1 is the last row

                // part6:
                // These are still referenced (but not used) in next rows
                region.assign_advice(
                    || "step height",
                    self.num_rows_until_next_step,
                    offset,
                    || Value::known(F::ZERO),
                )?;
                region.assign_advice(
                    || "step height inv",
                    self.q_step,
                    offset,
                    || Value::known(F::ZERO),
                )?;

                assign_pass += 1;
                Ok(offset)
            },
        )
    }

    fn annotate_circuit(&self, region: &mut Region<F>) {
        let groups = [
            ("EVM_lookup_fixed", FIXED_TABLE_LOOKUPS),
            ("EVM_lookup_tx", TX_TABLE_LOOKUPS),
            ("EVM_lookup_rw", RW_TABLE_LOOKUPS),
            ("EVM_lookup_bytecode", BYTECODE_TABLE_LOOKUPS),
            ("EVM_lookup_block", BLOCK_TABLE_LOOKUPS),
            ("EVM_lookup_copy", COPY_TABLE_LOOKUPS),
            ("EVM_lookup_keccak", KECCAK_TABLE_LOOKUPS),
            ("EVM_lookup_exp", EXP_TABLE_LOOKUPS),
            ("EVM_lookup_sig", SIG_TABLE_LOOKUPS),
            ("EVM_lookupchunk_ctx", CHUNK_CTX_TABLE_LOOKUPS),
            ("EVM_adv_phase2", N_PHASE2_COLUMNS),
            ("EVM_copy", N_COPY_COLUMNS),
            ("EVM_lookup_u8", N_U8_LOOKUPS),
            ("EVM_lookup_u16", N_U16_LOOKUPS),
            ("EVM_adv_phase1", N_PHASE1_COLUMNS),
        ];
        let mut group_index = 0;
        let mut index = 0;
        for col in self.advices {
            let (name, length) = groups[group_index];
            region.name_column(|| format!("{}_{}", name, index), col);
            index += 1;
            if index >= length {
                index = 0;
                group_index += 1;
            }
        }

        region.name_column(|| "EVM_q_step", self.q_step);
        region.name_column(|| "EVM_num_rows_inv", self.num_rows_inv);
        region.name_column(|| "EVM_rows_until_next_step", self.num_rows_until_next_step);
        region.name_column(|| "Copy_Constr_const", self.constants);
    }

    #[allow(clippy::too_many_arguments)]
    fn assign_same_exec_step_in_range(
        &self,
        region: &mut Region<'_, F>,
        offset_begin: usize,
        offset_end: usize,
        block: &Block<F>,
        chunk: &Chunk<F>,
        cur_step: TxCallStep,
        height: usize,
        challenges: &Challenges<Value<F>>,
        assign_pass: usize,
    ) -> Result<(), Error> {
        if offset_end <= offset_begin {
            return Ok(());
        }
        let (_, _, step) = cur_step;
        assert_eq!(height, 1);
        assert!(step.rw_indices_len() == 0);
        assert!(matches!(step.execution_state(), ExecutionState::Padding));

        // Disable access to next step deliberately for "repeatable" step
        let region = &mut CachedRegion::<'_, '_, F>::new(
            region,
            challenges,
            self.advices.to_vec(),
            1,
            offset_begin,
        );
        self.assign_exec_step_int(
            region,
            offset_begin,
            block,
            chunk,
            cur_step,
            false,
            assign_pass,
        )?;

        region.replicate_assignment_for_range(
            || format!("repeat {:?} rows", step.execution_state()),
            offset_begin + 1,
            offset_end,
        )?;

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        block: &Block<F>,
        chunk: &Chunk<F>,
        cur_step: TxCallStep,
        height: usize,
        next_step: Option<TxCallStep>,
        challenges: &Challenges<Value<F>>,
        assign_pass: usize,
    ) -> Result<(), Error> {
        let (_transaction, call, step) = cur_step;
        if !matches!(step.execution_state(), ExecutionState::Padding) {
            log::trace!(
                "assign_exec_step offset: {} state {:?} step: {:?} call: {:?}",
                offset,
                step.execution_state(),
                step,
                call
            );
        }
        // Make the region large enough for the current step and the next step.
        // The next step's next step may also be accessed, so make the region large
        // enough for 3 steps.
        let region = &mut CachedRegion::<'_, '_, F>::new(
            region,
            challenges,
            self.advices.to_vec(),
            MAX_STEP_HEIGHT * 3,
            offset,
        );

        // Also set the witness of the next step.
        // These may be used in stored expressions and
        // so their witness values need to be known to be able
        // to correctly calculate the intermediate value.
        if let Some(next_step) = next_step {
            self.assign_exec_step_int(
                region,
                offset + height,
                block,
                chunk,
                next_step,
                true,
                assign_pass,
            )?;
        }

        self.assign_exec_step_int(region, offset, block, chunk, cur_step, false, assign_pass)
    }

    #[allow(clippy::too_many_arguments)]
    fn assign_exec_step_int(
        &self,
        region: &mut CachedRegion<'_, '_, F>,
        offset: usize,
        block: &Block<F>,
        chunk: &Chunk<F>,
        tx_call_step: TxCallStep,
        // Set to true when we're assigning the next step before the current step to have
        // next step assignments for evaluation of the stored expressions in current step that
        // depend on the next step.
        is_next: bool,
        // Layouter assignment pass
        assign_pass: usize,
    ) -> Result<(), Error> {
        let (transaction, call, step) = tx_call_step;
        self.step
            .assign_exec_step(region, offset, block, call, step)?;

        macro_rules! assign_exec_step {
            ($gadget:expr) => {
                $gadget.assign_exec_step(region, offset, block, chunk, transaction, call, step)?
            };
        }

        match step.execution_state() {
            // internal states
            ExecutionState::BeginTx => assign_exec_step!(self.begin_tx_gadget),
            ExecutionState::EndTx => assign_exec_step!(self.end_tx_gadget),
            ExecutionState::Padding => assign_exec_step!(self.padding_gadget),
            ExecutionState::EndBlock => assign_exec_step!(self.end_block_gadget),
            ExecutionState::BeginChunk => assign_exec_step!(self.begin_chunk_gadget),
            ExecutionState::EndChunk => assign_exec_step!(self.end_chunk_gadget),
            ExecutionState::InvalidTx => {
                assign_exec_step!(self
                    .invalid_tx
                    .as_deref()
                    .expect("invalid tx gadget must exist"))
            }
            // opcode
            ExecutionState::ADD_SUB => assign_exec_step!(self.add_sub_gadget),
            ExecutionState::ADDMOD => assign_exec_step!(self.addmod_gadget),
            ExecutionState::ADDRESS => assign_exec_step!(self.address_gadget),
            ExecutionState::BALANCE => assign_exec_step!(self.balance_gadget),
            ExecutionState::BITWISE => assign_exec_step!(self.bitwise_gadget),
            ExecutionState::BYTE => assign_exec_step!(self.byte_gadget),
            ExecutionState::CALL_OP => assign_exec_step!(self.call_op_gadget),
            ExecutionState::CALLDATACOPY => assign_exec_step!(self.calldatacopy_gadget),
            ExecutionState::CALLDATALOAD => assign_exec_step!(self.calldataload_gadget),
            ExecutionState::CALLDATASIZE => assign_exec_step!(self.calldatasize_gadget),
            ExecutionState::CALLER => assign_exec_step!(self.caller_gadget),
            ExecutionState::CALLVALUE => assign_exec_step!(self.call_value_gadget),
            ExecutionState::CHAINID => assign_exec_step!(self.chainid_gadget),
            ExecutionState::CODECOPY => assign_exec_step!(self.codecopy_gadget),
            ExecutionState::CODESIZE => assign_exec_step!(self.codesize_gadget),
            ExecutionState::CMP => assign_exec_step!(self.comparator_gadget),
            ExecutionState::DUP => assign_exec_step!(self.dup_gadget),
            ExecutionState::EXP => assign_exec_step!(self.exp_gadget),
            ExecutionState::EXTCODEHASH => assign_exec_step!(self.extcodehash_gadget),
            ExecutionState::EXTCODESIZE => assign_exec_step!(self.extcodesize_gadget),
            ExecutionState::GAS => assign_exec_step!(self.gas_gadget),
            ExecutionState::GASPRICE => assign_exec_step!(self.gasprice_gadget),
            ExecutionState::ISZERO => assign_exec_step!(self.iszero_gadget),
            ExecutionState::JUMP => assign_exec_step!(self.jump_gadget),
            ExecutionState::JUMPDEST => assign_exec_step!(self.jumpdest_gadget),
            ExecutionState::JUMPI => assign_exec_step!(self.jumpi_gadget),
            ExecutionState::LOG => assign_exec_step!(self.log_gadget),
            ExecutionState::MEMORY => assign_exec_step!(self.memory_gadget),
            ExecutionState::MSIZE => assign_exec_step!(self.msize_gadget),
            ExecutionState::MUL_DIV_MOD => assign_exec_step!(self.mul_div_mod_gadget),
            ExecutionState::MULMOD => assign_exec_step!(self.mulmod_gadget),
            ExecutionState::NOT => assign_exec_step!(self.not_gadget),
            ExecutionState::ORIGIN => assign_exec_step!(self.origin_gadget),
            ExecutionState::PC => assign_exec_step!(self.pc_gadget),
            ExecutionState::POP => assign_exec_step!(self.pop_gadget),
            ExecutionState::PUSH => assign_exec_step!(self.push_gadget),
            ExecutionState::RETURN_REVERT => assign_exec_step!(self.return_revert_gadget),
            ExecutionState::RETURNDATASIZE => assign_exec_step!(self.returndatasize_gadget),
            ExecutionState::RETURNDATACOPY => assign_exec_step!(self.returndatacopy_gadget),
            ExecutionState::SAR => assign_exec_step!(self.sar_gadget),
            ExecutionState::SCMP => assign_exec_step!(self.signed_comparator_gadget),
            ExecutionState::SDIV_SMOD => assign_exec_step!(self.sdiv_smod_gadget),
            ExecutionState::BLOCKCTX => assign_exec_step!(self.block_ctx_gadget),
            ExecutionState::BLOCKHASH => assign_exec_step!(self.blockhash_gadget),
            ExecutionState::SELFBALANCE => assign_exec_step!(self.selfbalance_gadget),
            // dummy gadgets
            ExecutionState::EXTCODECOPY => assign_exec_step!(self.extcodecopy_gadget),
            ExecutionState::CREATE => assign_exec_step!(self.create_gadget),
            ExecutionState::CREATE2 => assign_exec_step!(self.create2_gadget),
            ExecutionState::SELFDESTRUCT => assign_exec_step!(self.selfdestruct_gadget),
            // end of dummy gadgets
            ExecutionState::SHA3 => assign_exec_step!(self.sha3_gadget),
            ExecutionState::SHL_SHR => assign_exec_step!(self.shl_shr_gadget),
            ExecutionState::SIGNEXTEND => assign_exec_step!(self.signextend_gadget),
            ExecutionState::SLOAD => assign_exec_step!(self.sload_gadget),
            ExecutionState::SSTORE => assign_exec_step!(self.sstore_gadget),
            ExecutionState::TLOAD => assign_exec_step!(self.tload_gadget),
            ExecutionState::TSTORE => assign_exec_step!(self.tstore_gadget),
            ExecutionState::STOP => assign_exec_step!(self.stop_gadget),
            ExecutionState::SWAP => assign_exec_step!(self.swap_gadget),
            // dummy errors
            ExecutionState::ErrorOutOfGasStaticMemoryExpansion => {
                assign_exec_step!(self.error_oog_static_memory_gadget)
            }
            ExecutionState::ErrorOutOfGasConstant => {
                assign_exec_step!(self.error_oog_constant)
            }
            ExecutionState::ErrorOutOfGasCall => {
                assign_exec_step!(self.error_oog_call)
            }
            ExecutionState::ErrorOutOfGasPrecompile => {
                assign_exec_step!(self.error_oog_precompile)
            }
            ExecutionState::ErrorOutOfGasDynamicMemoryExpansion => {
                assign_exec_step!(self.error_oog_dynamic_memory_gadget)
            }
            ExecutionState::ErrorOutOfGasLOG => {
                assign_exec_step!(self.error_oog_log)
            }
            ExecutionState::ErrorOutOfGasSloadSstore => {
                assign_exec_step!(self.error_oog_sload_sstore)
            }
            ExecutionState::ErrorOutOfGasMemoryCopy => {
                assign_exec_step!(self.error_oog_memory_copy)
            }
            ExecutionState::ErrorOutOfGasAccountAccess => {
                assign_exec_step!(self.error_oog_account_access)
            }
            ExecutionState::ErrorOutOfGasSHA3 => {
                assign_exec_step!(self.error_oog_sha3)
            }
            ExecutionState::ErrorOutOfGasEXTCODECOPY => {
                assign_exec_step!(self.error_oog_ext_codecopy)
            }
            ExecutionState::ErrorOutOfGasEXP => {
                assign_exec_step!(self.error_oog_exp)
            }
            ExecutionState::ErrorOutOfGasCREATE => {
                assign_exec_step!(self.error_oog_create)
            }
            ExecutionState::ErrorOutOfGasSELFDESTRUCT => {
                assign_exec_step!(self.error_oog_self_destruct)
            }

            ExecutionState::ErrorCodeStore => {
                assign_exec_step!(self.error_oog_code_store)
            }
            ExecutionState::ErrorStack => {
                assign_exec_step!(self.error_stack)
            }

            ExecutionState::ErrorInvalidJump => {
                assign_exec_step!(self.error_invalid_jump)
            }
            ExecutionState::ErrorInvalidOpcode => {
                assign_exec_step!(self.error_invalid_opcode)
            }
            ExecutionState::ErrorWriteProtection => {
                assign_exec_step!(self.error_write_protection)
            }
            ExecutionState::ErrorInvalidCreationCode => {
                assign_exec_step!(self.error_invalid_creation_code)
            }
            ExecutionState::ErrorReturnDataOutOfBound => {
                assign_exec_step!(self.error_return_data_out_of_bound)
            }
            ExecutionState::ErrorPrecompileFailed => {
                assign_exec_step!(self.error_precompile_failed)
            }
            ExecutionState::PrecompileEcrecover => {
                assign_exec_step!(self.precompile_ecrecover_gadget)
            }
            ExecutionState::PrecompileIdentity => {
                assign_exec_step!(self.precompile_identity_gadget)
            }

            unimpl_state => evm_unimplemented!("unimplemented ExecutionState: {:?}", unimpl_state),
        }

        // Fill in the witness values for stored expressions
        let assigned_stored_expressions = self.assign_stored_expressions(region, offset, step)?;
        // Both `SimpleFloorPlanner` and `V1` do two passes; we only enter here once (on the second
        // pass).
        if !is_next && assign_pass == 1 {
            // We only want to print at the latest possible Phase.  Currently halo2 implements 3
            // phases.  The `lookup_input` randomness is calculated after SecondPhase, so we print
            // the debug expressions only once when we're at third phase, when `lookup_input` has
            // a `Value::known`.  This gets called for every `synthesize` call that the Layouter
            // does.
            region.challenges().lookup_input().assert_if_known(|_| {
                self.print_debug_expressions(region, offset, step);
                true
            });

            // enable with `RUST_LOG=debug`
            if log::log_enabled!(log::Level::Debug) {
                let is_padding_step = matches!(step.execution_state(), ExecutionState::Padding);
                if !is_padding_step {
                    // expensive function call
                    Self::check_rw_lookup(
                        &assigned_stored_expressions,
                        step,
                        block,
                        chunk,
                        region.challenges(),
                    );
                }
            }
        }
        Ok(())
    }

    fn assign_stored_expressions(
        &self,
        region: &mut CachedRegion<'_, '_, F>,
        offset: usize,
        step: &ExecStep,
    ) -> Result<Vec<(String, F)>, Error> {
        let mut assigned_stored_expressions = Vec::new();
        for stored_expression in self
            .stored_expressions_map
            .get(&step.execution_state())
            .unwrap_or_else(|| panic!("Execution state unknown: {:?}", step.execution_state()))
        {
            let assigned = stored_expression.assign(region, offset)?;
            assigned.map(|v| {
                let name = stored_expression.name.clone();
                assigned_stored_expressions.push((name, v));
            });
        }
        Ok(assigned_stored_expressions)
    }

    fn print_debug_expressions(
        &self,
        region: &mut CachedRegion<'_, '_, F>,
        offset: usize,
        step: &ExecStep,
    ) {
        for (name, expression) in self
            .debug_expressions_map
            .get(&step.execution_state())
            .unwrap_or_else(|| panic!("Execution state unknown: {:?}", step.execution_state()))
        {
            let value = evaluate_expression(expression, region, offset);
            let mut value_string = "unknown".to_string();
            value.assert_if_known(|f| {
                value_string = format!("{:?}", f);
                true
            });
            println!(
                "Debug expression \"{}\"={} [offset={}, step={:?}, expr={:?}]",
                name, value_string, offset, step.exec_state, expression
            );
        }
    }

    fn check_rw_lookup(
        assigned_stored_expressions: &[(String, F)],
        step: &ExecStep,
        block: &Block<F>,
        _chunk: &Chunk<F>,
        challenges: &Challenges<Value<F>>,
    ) {
        let mut lookup_randomness = F::ZERO;
        challenges.lookup_input().map(|v| lookup_randomness = v);
        if lookup_randomness.is_zero_vartime() {
            // challenges not ready
            return;
        }

        let mut copy_lookup_count = 0;
        let mut assigned_rw_values = Vec::new();
        for (name, v) in assigned_stored_expressions {
            // If any `copy lookup` which dst_tag or src_tag is Memory in opcode execution,
            // block.get_rws() contains memory operations but
            // assigned_stored_expressions only has a single `copy lookup` expression without
            // any rw memory lookup.
            // So, we include `copy lookup` in assigned_rw_values as well, then we could verify
            // those memory operations later.
            if (name.starts_with("rw lookup ") || name.starts_with("copy lookup"))
                && !v.is_zero_vartime()
                && !assigned_rw_values.contains(&(name.clone(), *v))
            {
                assigned_rw_values.push((name.clone(), *v));

                if name.starts_with("copy lookup") {
                    copy_lookup_count += 1;
                }
            }
        }

        // TODO: We should find a better way to avoid this kind of special case.
        // #1489 is the issue for this refactor.
        if copy_lookup_count > 1 {
            log::warn!("The number of copy events is more than 1, it's not supported by current design. Stop checking this step: {:?}",
                step
            );
            return;
        }

        let rlc_assignments: BTreeSet<_> = (0..step.rw_indices_len())
            .map(|index| block.get_rws(step, index))
            .map(|rw| rw.table_assignment().unwrap().rlc(lookup_randomness))
            .fold(BTreeSet::<F>::new(), |mut set, value| {
                set.insert(value);
                set
            });

        // Check that every rw_lookup assigned from the execution steps in the EVM
        // Circuit is in the set of rw operations generated by the step.
        for (name, value) in assigned_rw_values.iter() {
            if name.starts_with("copy lookup") {
                continue;
            }
            if !rlc_assignments.contains(value) {
                log::error!("rw lookup error: name: {}, step: {:?}", *name, step);
            }
        }

        // if copy_rw_counter_delta is zero, ignore `copy lookup` event.
        if step.copy_rw_counter_delta == 0 && copy_lookup_count > 0 {
            copy_lookup_count = 0;
        }

        // Check that the number of rw operations generated from the bus-mapping
        // correspond to the number of assigned rw lookups by the EVM Circuit
        // plus the number of rw lookups done by the copy circuit
        // minus the number of copy lookup event.
        if step.rw_indices_len()
            != assigned_rw_values.len() + step.copy_rw_counter_delta as usize - copy_lookup_count
        {
            log::error!(
                "step.rw_indices.len: {} != assigned_rw_values.len: {} + step.copy_rw_counter_delta: {} - copy_lookup_count: {} in step: {:?}",
                step.rw_indices_len(),
                assigned_rw_values.len(),
                step.copy_rw_counter_delta,
                copy_lookup_count,
                step
            );
        }

        let mut rev_count = 0;
        let mut offset = 0;
        let mut copy_lookup_processed = false;
        for (idx, assigned_rw_value) in assigned_rw_values.iter().enumerate() {
            let is_rev = if assigned_rw_value.0.contains(" with reversion") {
                rev_count += 1;
                true
            } else {
                false
            };
            assert!(
                rev_count <= step.reversible_write_counter_delta,
                "Assigned {} reversions, but step only has {}",
                rev_count,
                step.reversible_write_counter_delta
            );

            // In the EVM Circuit, reversion rw lookups are assigned after their
            // corresponding rw lookup, but in the bus-mapping they are
            // generated at the end of the step.
            let idx = if is_rev {
                step.rw_indices_len() - rev_count
            } else {
                idx - rev_count + offset - copy_lookup_processed as usize
            };

            // If assigned_rw_value is a `copy lookup` event, the following
            // `step.copy_rw_counter_delta` rw lookups must be memory operations.
            if assigned_rw_value.0.starts_with("copy lookup") {
                for i in 0..step.copy_rw_counter_delta as usize {
                    let index = idx + i;
                    let rw = block.get_rws(step, index);
                    if rw.tag() != Target::Memory {
                        log::error!(
                                "incorrect rw memory witness from copy lookup.\n lookup name: \"{}\"\n {}th rw of step {:?}, rw: {:?}",
                                assigned_rw_value.0,
                                index,
                                step.execution_state(),
                                rw);
                    }
                }

                offset = step.copy_rw_counter_delta as usize;
                copy_lookup_processed = true;
                continue;
            }

            let rw = block.get_rws(step, idx);
            let table_assignments = rw.table_assignment();
            let rlc = table_assignments.unwrap().rlc(lookup_randomness);
            if rlc != assigned_rw_value.1 {
                log::error!(
                    "incorrect rw witness. lookup input name: \"{}\"\nassigned={:?}\nrlc     ={:?}\n{}th rw of step {:?}, rw: {:?}",
                    assigned_rw_value.0,
                    assigned_rw_value.1,
                    rlc,
                    idx,
                    step.execution_state(),
                    rw);
            }
        }
    }
}
