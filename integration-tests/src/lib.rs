//! Cw-multi-test harness for the v1 Astroport-Juno keep-set.
//!
//! This crate is the composability proof + deploy-runbook-in-code for
//! the contracts at `v0.1.1-juno-rc1` (and now `v0.1.2-juno-rc2` after
//! P2.5 lands the incentives contract). The shared `deploy_keep_set()`
//! helper here mirrors the sequence that `planning/06-deploy-runbook.md`
//! (to be written in P5) describes for `junod tx wasm store` /
//! instantiate against uni-7 and juno-1.
//!
//! Per-contract integration coverage continues to live in each contract
//! crate's `tests/` directory. The test files here under `tests/` only
//! assert *composability* across the keep set:
//!
//! - `deploy_sequence.rs` — full keep-set deploy + TF LP denom shape.
//! - `multi_hop_routing.rs` — router smoke with Juno-realistic denoms.
//! - `paused_via_factory.rs` — pool_unpause_at plumbed through
//!   factory.CreatePair (the wire path the cw-abc graduation flow needs).
//! - `incentives_setup_pools.rs` — admin/controller sets alloc_points;
//!   LP stakes; rewards accrue; ClaimRewards harvests (P2.5).
//! - `incentives_external_native.rs` — third party funds a pool with a
//!   non-ujuno native reward (P2.5).
//! - `incentives_external_cw20.rs` — third party funds a pool with a cw20
//!   reward token. AUDIT regression gate: proves the cw20-LP strip
//!   didn't break the cw20-as-reward-token path (P2.5).
//!
//! **Bech32 mode** (added 2026-05-13): the app uses `MockApiBech32` with
//! prefix `"juno"` so that `addr_validate("factory/juno1xxx/astroport/share")`
//! correctly fails (slashes aren't valid bech32), forcing the TF LP denom
//! to be parsed as `AssetInfo::NativeToken` rather than mis-classified as
//! a cw20 contract address. The production chain has bech32 addresses so
//! this matches real-world behavior.

use anyhow::Result as AnyResult;
use cosmwasm_std::{coin, testing::MockStorage, Addr, Coin, Empty, Uint128};

use astroport::asset::AssetInfo;
use astroport::factory::{InstantiateMsg as FactoryInstantiateMsg, PairConfig, PairType};
use astroport::incentives::{IncentivizationFeeInfo, InstantiateMsg as IncentivesInstantiateMsg};
use astroport::native_coin_registry::{
    ExecuteMsg as RegistryExecuteMsg, InstantiateMsg as RegistryInstantiateMsg,
};
use astroport::router::InstantiateMsg as RouterInstantiateMsg;
use astroport_test::cw_multi_test::{
    App, AppBuilder, BankKeeper, ContractWrapper, DistributionKeeper, Executor, FailingModule,
    GovFailingModule, IbcFailingModule, MockAddressGenerator, MockApiBech32, StakeKeeper,
    WasmKeeper,
};
use astroport_test::modules::stargate::MockStargate;

/// Bech32 prefix used by the test app's address validator. Matches the
/// production Juno chain.
pub const BECH32_PREFIX: &str = "juno";

/// Canonical short-name for the deployer in helper calls. The actual
/// bech32 address is derived via `app.api().addr_make(DEPLOYER)`.
pub const DEPLOYER: &str = "deployer";

/// Realistic Juno-style denoms used across all tests. The IBC paths
/// are mocked but the bech32-prefix / precision conventions match what
/// will be in juno-1 after deploy.
pub const UJUNO: &str = "ujuno";
pub const MOCK_USDC: &str = "ibc/USDC";
pub const MOCK_ATOM: &str = "ibc/ATOM";

/// Initial bank balance per deployer-side denom. Generous enough that
/// every test in the harness can seed pools + do follow-on flows.
pub const INITIAL_BALANCE: u128 = 1_000_000_000_000;

/// The cw-multi-test app type used by all integration tests in this
/// crate. Differs from `astroport_test::modules::stargate::StargateApp`
/// only in `Api`: this uses `MockApiBech32("juno")` so TF LP denoms
/// disambiguate correctly from cw20 contract addresses.
pub type TestApp = App<
    BankKeeper,
    MockApiBech32,
    MockStorage,
    FailingModule<Empty, Empty, Empty>,
    WasmKeeper<Empty, Empty>,
    StakeKeeper,
    DistributionKeeper,
    IbcFailingModule,
    GovFailingModule,
    MockStargate,
>;

/// Handles to the instantiated keep-set contracts + the code IDs needed
/// to instantiate further pairs.
pub struct KeepSetHandles {
    pub deployer: Addr,
    pub factory: Addr,
    pub native_coin_registry: Addr,
    pub whitelist: Addr,
    pub router: Addr,
    pub pair_code_id: u64,
    pub whitelist_code_id: u64,
    pub token_code_id: u64,
}

/// Bootstrap a `MockStargate` cw-multi-test app in bech32 mode with
/// Juno-style bank balances pre-seeded for the deployer. Uses
/// `MockAddressGenerator` so freshly-instantiated contract addresses
/// are bech32-valid (the default `SimpleAddressGenerator` produces
/// `"contract0"` etc. which fail `addr_validate` in bech32 mode).
pub fn mock_app() -> TestApp {
    let api = MockApiBech32::new(BECH32_PREFIX);
    let deployer = api.addr_make(DEPLOYER);
    let coins = vec![
        coin(INITIAL_BALANCE, UJUNO),
        coin(INITIAL_BALANCE, MOCK_USDC),
        coin(INITIAL_BALANCE, MOCK_ATOM),
    ];
    let wasm = WasmKeeper::new().with_address_generator(MockAddressGenerator);
    AppBuilder::new_custom()
        .with_api(api)
        .with_wasm(wasm)
        .with_stargate(MockStargate::default())
        .build(|router, _, storage| router.bank.init_balance(storage, &deployer, coins).unwrap())
}

fn store_factory_code(app: &mut TestApp) -> u64 {
    app.store_code(Box::new(
        ContractWrapper::new_with_empty(
            astroport_factory::contract::execute,
            astroport_factory::contract::instantiate,
            astroport_factory::contract::query,
        )
        .with_reply_empty(astroport_factory::contract::reply),
    ))
}

fn store_pair_code(app: &mut TestApp) -> u64 {
    app.store_code(Box::new(
        ContractWrapper::new_with_empty(
            astroport_pair::contract::execute,
            astroport_pair::contract::instantiate,
            astroport_pair::contract::query,
        )
        .with_reply_empty(astroport_pair::contract::reply),
    ))
}

fn store_router_code(app: &mut TestApp) -> u64 {
    app.store_code(Box::new(
        ContractWrapper::new_with_empty(
            astroport_router::contract::execute,
            astroport_router::contract::instantiate,
            astroport_router::contract::query,
        )
        .with_reply_empty(astroport_router::contract::reply),
    ))
}

fn store_whitelist_code(app: &mut TestApp) -> u64 {
    app.store_code(Box::new(ContractWrapper::new_with_empty(
        astroport_whitelist::contract::execute,
        astroport_whitelist::contract::instantiate,
        astroport_whitelist::contract::query,
    )))
}

fn store_registry_code(app: &mut TestApp) -> u64 {
    app.store_code(Box::new(ContractWrapper::new_with_empty(
        astroport_native_coin_registry::contract::execute,
        astroport_native_coin_registry::contract::instantiate,
        astroport_native_coin_registry::contract::query,
    )))
}

fn store_cw20_code(app: &mut TestApp) -> u64 {
    app.store_code(Box::new(ContractWrapper::new_with_empty(
        cw20_base::contract::execute,
        cw20_base::contract::instantiate,
        cw20_base::contract::query,
    )))
}

/// Deploy the v1 keep-set. Mirrors `planning/06-deploy-runbook.md`:
///
/// 1. `store_code` for the 5 contracts + cw20-base (test-side LP-token
///    placeholder; not actually used by the pair, which mints TF).
/// 2. Instantiate `native_coin_registry`; register the three Juno-style
///    denoms with their precisions.
/// 3. Instantiate `whitelist` (no admin gating in v1, but the contract
///    ships uploaded so a future PairConfig can flip `permissioned`).
/// 4. Instantiate `factory` with the registry address + the XYK pair
///    code_id. `total_fee_bps: 30, maker_fee_bps: 0, permissioned: false`
///    per the v1 fee-defaults.
/// 5. Instantiate `router` with the factory address.
pub fn deploy_keep_set(app: &mut TestApp) -> AnyResult<KeepSetHandles> {
    let deployer = app.api().addr_make(DEPLOYER);

    let pair_code_id = store_pair_code(app);
    let factory_code_id = store_factory_code(app);
    let router_code_id = store_router_code(app);
    let whitelist_code_id = store_whitelist_code(app);
    let registry_code_id = store_registry_code(app);
    let token_code_id = store_cw20_code(app);

    // 1. native_coin_registry
    let native_coin_registry = app.instantiate_contract(
        registry_code_id,
        deployer.clone(),
        &RegistryInstantiateMsg {
            owner: deployer.to_string(),
        },
        &[],
        "native_coin_registry",
        None,
    )?;
    app.execute_contract(
        deployer.clone(),
        native_coin_registry.clone(),
        &RegistryExecuteMsg::Add {
            native_coins: vec![
                (UJUNO.to_string(), 6),
                (MOCK_USDC.to_string(), 6),
                (MOCK_ATOM.to_string(), 6),
            ],
        },
        &[],
    )?;

    // 2. whitelist — vanilla cw1 (Neutron-stripped). admins are stable
    //    across the harness so a future test can rely on the deployer
    //    being on the list.
    let whitelist = app.instantiate_contract(
        whitelist_code_id,
        deployer.clone(),
        &cw1_whitelist::msg::InstantiateMsg {
            admins: vec![deployer.to_string()],
            mutable: true,
        },
        &[],
        "whitelist",
        None,
    )?;

    // 3. factory
    let factory = app.instantiate_contract(
        factory_code_id,
        deployer.clone(),
        &FactoryInstantiateMsg {
            pair_configs: vec![PairConfig {
                code_id: pair_code_id,
                pair_type: PairType::Xyk {},
                total_fee_bps: 30,
                maker_fee_bps: 0,
                is_disabled: false,
                // is_generator_disabled = false: XYK pools are eligible
                // to receive incentives. Flipped from upstream's default
                // because incentives is v1 scope (P2.5). Pools still
                // only emit when explicitly registered via SetupPools.
                is_generator_disabled: false,
                permissioned: false,
                whitelist: None,
            }],
            token_code_id,
            fee_address: None,
            generator_address: None,
            owner: deployer.to_string(),
            whitelist_code_id,
            coin_registry_address: native_coin_registry.to_string(),
            tracker_config: None,
        },
        &[],
        "factory",
        None,
    )?;

    // 4. router
    let router = app.instantiate_contract(
        router_code_id,
        deployer.clone(),
        &RouterInstantiateMsg {
            astroport_factory: factory.to_string(),
        },
        &[],
        "router",
        None,
    )?;

    Ok(KeepSetHandles {
        deployer,
        factory,
        native_coin_registry,
        whitelist,
        router,
        pair_code_id,
        whitelist_code_id,
        token_code_id,
    })
}

/// Transfer native funds from the deployer to a fresh test wallet.
/// Convenience wrapper around `app.send_tokens`.
pub fn fund(app: &mut TestApp, to: &Addr, amount: Vec<Coin>) -> AnyResult<()> {
    let deployer = app.api().addr_make(DEPLOYER);
    app.send_tokens(deployer, to.clone(), &amount)?;
    Ok(())
}

/// Query a native-denom balance of `who`. Returns 0 if absent.
pub fn balance_of(app: &TestApp, who: &Addr, denom: &str) -> Uint128 {
    app.wrap()
        .query_balance(who, denom)
        .map(|c| c.amount)
        .unwrap_or_default()
}

/// Handle returned by [`deploy_incentives_addon`]. Adds the incentives
/// contract to a keep-set that's already been deployed.
pub struct IncentivesHandle {
    pub incentives: Addr,
    pub incentives_code_id: u64,
}

fn store_incentives_code(app: &mut TestApp) -> u64 {
    app.store_code(Box::new(
        ContractWrapper::new_with_empty(
            astroport_incentives::execute::execute,
            astroport_incentives::instantiate::instantiate,
            astroport_incentives::query::query,
        )
        .with_reply_empty(astroport_incentives::reply::reply),
    ))
}

/// Deploy the post-strip astroport-incentives contract against the
/// existing keep-set. The deployer becomes the owner; `reward_token`
/// is the internal (DAO-funded) reward emission token (typically
/// `AssetInfo::native("ujuno")` for the production deploy).
///
/// `incentivization_fee_info` defaults to None (no spam fee) for ease
/// of testing; the production deploy sets this to ~100 ujuno via
/// UpdateConfig after instantiate.
pub fn deploy_incentives_addon(
    app: &mut TestApp,
    handles: &KeepSetHandles,
    reward_token: AssetInfo,
    incentivization_fee_info: Option<IncentivizationFeeInfo>,
) -> AnyResult<IncentivesHandle> {
    let incentives_code_id = store_incentives_code(app);
    let incentives = app.instantiate_contract(
        incentives_code_id,
        handles.deployer.clone(),
        &IncentivesInstantiateMsg {
            owner: handles.deployer.to_string(),
            factory: handles.factory.to_string(),
            reward_token,
            incentivization_fee_info,
            guardian: None,
        },
        &[],
        "astroport-incentives",
        Some(handles.deployer.to_string()),
    )?;
    Ok(IncentivesHandle {
        incentives,
        incentives_code_id,
    })
}
