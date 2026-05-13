//! Cw-multi-test harness for the v1 Astroport-Juno keep-set.
//!
//! This crate is the composability proof + deploy-runbook-in-code for
//! the contracts at `v0.1.1-juno-rc1`. The shared `deploy_keep_set()`
//! helper here mirrors the sequence that `planning/06-deploy-runbook.md`
//! (to be written in P5) describes for `junod tx wasm store` / instantiate
//! against uni-7 and juno-1.
//!
//! Per-contract integration coverage continues to live in each contract
//! crate's `tests/` directory. The test files here under `tests/` only
//! assert *composability* across the keep set:
//!
//! - `deploy_sequence.rs` — full keep-set deploy + TF LP denom shape.
//! - `multi_hop_routing.rs` — router smoke with Juno-realistic denoms.
//! - `paused_via_factory.rs` — pool_unpause_at plumbed through
//!   factory.CreatePair (the wire path the cw-abc graduation flow needs).

use anyhow::Result as AnyResult;
use cosmwasm_std::{coin, Addr, Coin, Uint128};

use astroport::factory::{InstantiateMsg as FactoryInstantiateMsg, PairConfig, PairType};
use astroport::native_coin_registry::{
    ExecuteMsg as RegistryExecuteMsg, InstantiateMsg as RegistryInstantiateMsg,
};
use astroport::router::InstantiateMsg as RouterInstantiateMsg;
use astroport_test::cw_multi_test::{AppBuilder, ContractWrapper, Executor};
use astroport_test::modules::stargate::{MockStargate, StargateApp as TestApp};

/// The deployer address used by all keep-set integration tests.
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

/// Bootstrap a `MockStargate` cw-multi-test app with realistic
/// Juno-style bank balances pre-seeded for `DEPLOYER`.
pub fn mock_app() -> TestApp {
    let deployer = Addr::unchecked(DEPLOYER);
    let coins = vec![
        coin(INITIAL_BALANCE, UJUNO),
        coin(INITIAL_BALANCE, MOCK_USDC),
        coin(INITIAL_BALANCE, MOCK_ATOM),
    ];
    AppBuilder::new_custom()
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
    let deployer = Addr::unchecked(DEPLOYER);

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
                is_generator_disabled: true,
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
    let deployer = Addr::unchecked(DEPLOYER);
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
