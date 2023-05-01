use std::collections::HashMap;
use std::env;
use std::thread;

use stacks::burnchains::Burnchain;
use stacks::core::PEER_VERSION_EPOCH_2_2;
use stacks::core::PEER_VERSION_EPOCH_2_3;
use stacks::core::STACKS_EPOCH_MAX;
use stacks::vm::types::QualifiedContractIdentifier;

use crate::config::EventKeyType;
use crate::config::EventObserverConfig;
use crate::config::InitialBalance;
use crate::neon;
use crate::tests::bitcoin_regtest::BitcoinCoreController;
use crate::tests::neon_integrations::*;
use crate::tests::*;
use crate::BitcoinRegtestController;
use crate::BurnchainController;
use stacks::core;

use stacks::burnchains::PoxConstants;

use clarity::vm::types::PrincipalData;

#[test]
#[ignore]
/// Test the trait invocation behavior for contracts instantiated in epoch 2.05
///  * in epoch 2.1: the trait invocation works
///  * in epoch 2.2: trait invocation is broken, and returns a runtime error, even when wrapped
///  * in epoch 2.3: the trait invocation works
fn trait_invocation_behavior() {
    if env::var("BITCOIND_TEST") != Ok("1".into()) {
        return;
    }

    let reward_cycle_len = 10;
    let prepare_phase_len = 3;
    let epoch_2_05 = 215;
    let epoch_2_1 = 230;
    let v1_unlock_height = 231;
    let epoch_2_2 = 235;
    let epoch_2_3 = 241;

    let spender_sk = StacksPrivateKey::new();
    let contract_addr = to_addr(&spender_sk);
    let spender_addr: PrincipalData = to_addr(&spender_sk).into();

    let impl_contract_id =
        QualifiedContractIdentifier::new(contract_addr.clone().into(), "impl-simple".into());

    let mut spender_nonce = 0;
    let fee_amount = 10_000;

    let mut initial_balances = vec![];

    initial_balances.push(InitialBalance {
        address: spender_addr.clone(),
        amount: 1_000_000,
    });

    let trait_contract = "(define-trait simple-method ((foo (uint) (response uint uint)) ))";
    let impl_contract =
        "(impl-trait .simple-trait.simple-method) (define-read-only (foo (x uint)) (ok x))";
    let use_contract = "(use-trait simple .simple-trait.simple-method)
                        (define-public (call-simple (s <simple>)) (contract-call? s foo u0))";
    let invoke_contract = "
        (use-trait simple .simple-trait.simple-method)
        (define-public (invocation-1)
          (contract-call? .use-simple call-simple .impl-simple))
        (define-public (invocation-2 (st <simple>))
          (contract-call? .use-simple call-simple st))
    ";

    let wrapper_contract = "
        (use-trait simple .simple-trait.simple-method)
        (define-public (invocation-1)
          (contract-call? .invoke-simple invocation-1))
        (define-public (invocation-2 (st <simple>))
          (contract-call? .invoke-simple invocation-2 st))
    ";

    let (mut conf, _) = neon_integration_test_conf();

    conf.node.mine_microblocks = false;
    conf.burnchain.max_rbf = 1000000;
    conf.node.wait_time_for_microblocks = 0;
    conf.node.microblock_frequency = 1_000;
    conf.miner.first_attempt_time_ms = 2_000;
    conf.miner.subsequent_attempt_time_ms = 5_000;
    conf.node.wait_time_for_blocks = 1_000;
    conf.miner.wait_for_block_download = false;

    conf.miner.min_tx_fee = 1;
    conf.miner.first_attempt_time_ms = i64::max_value() as u64;
    conf.miner.subsequent_attempt_time_ms = i64::max_value() as u64;

    test_observer::spawn();

    conf.events_observers.push(EventObserverConfig {
        endpoint: format!("localhost:{}", test_observer::EVENT_OBSERVER_PORT),
        events_keys: vec![EventKeyType::AnyEvent],
    });
    conf.initial_balances.append(&mut initial_balances);

    let mut epochs = core::STACKS_EPOCHS_REGTEST.to_vec();
    epochs[1].end_height = epoch_2_05;
    epochs[2].start_height = epoch_2_05;
    epochs[2].end_height = epoch_2_1;
    epochs[3].start_height = epoch_2_1;
    epochs[3].end_height = epoch_2_2;
    epochs.push(StacksEpoch {
        epoch_id: StacksEpochId::Epoch22,
        start_height: epoch_2_2,
        end_height: epoch_2_3,
        block_limit: epochs[3].block_limit.clone(),
        network_epoch: PEER_VERSION_EPOCH_2_2,
    });
    epochs.push(StacksEpoch {
        epoch_id: StacksEpochId::Epoch23,
        start_height: epoch_2_3,
        end_height: STACKS_EPOCH_MAX,
        block_limit: epochs[3].block_limit.clone(),
        network_epoch: PEER_VERSION_EPOCH_2_3,
    });
    conf.burnchain.epochs = Some(epochs);

    let mut burnchain_config = Burnchain::regtest(&conf.get_burn_db_path());

    let pox_constants = PoxConstants::new(
        reward_cycle_len,
        prepare_phase_len,
        4 * prepare_phase_len / 5,
        5,
        15,
        u64::max_value() - 2,
        u64::max_value() - 1,
        v1_unlock_height as u32,
        epoch_2_2 as u32 + 1,
    );
    burnchain_config.pox_constants = pox_constants.clone();

    let mut btcd_controller = BitcoinCoreController::new(conf.clone());
    btcd_controller
        .start_bitcoind()
        .map_err(|_e| ())
        .expect("Failed starting bitcoind");

    let mut btc_regtest_controller = BitcoinRegtestController::with_burnchain(
        conf.clone(),
        None,
        Some(burnchain_config.clone()),
        None,
    );
    let http_origin = format!("http://{}", &conf.node.rpc_bind);

    btc_regtest_controller.bootstrap_chain(201);

    eprintln!("Chain bootstrapped...");

    let mut run_loop = neon::RunLoop::new(conf.clone());
    let runloop_burnchain = burnchain_config.clone();

    let blocks_processed = run_loop.get_blocks_processed_arc();

    let channel = run_loop.get_coordinator_channel().unwrap();

    thread::spawn(move || run_loop.start(Some(runloop_burnchain), 0));

    // give the run loop some time to start up!
    wait_for_runloop(&blocks_processed);

    // first block wakes up the run loop
    next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);

    // first block will hold our VRF registration
    next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);

    // second block will be the first mined Stacks block
    next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);

    // push us to block 205
    next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);

    // publish contracts right away!
    let publish_trait = make_contract_publish(
        &spender_sk,
        spender_nonce,
        fee_amount,
        "simple-trait",
        trait_contract,
    );

    spender_nonce += 1;

    let publish_impl = make_contract_publish(
        &spender_sk,
        spender_nonce,
        fee_amount,
        "impl-simple",
        impl_contract,
    );

    spender_nonce += 1;

    let publish_use = make_contract_publish(
        &spender_sk,
        spender_nonce,
        fee_amount,
        "use-simple",
        use_contract,
    );

    spender_nonce += 1;

    let publish_invoke = make_contract_publish(
        &spender_sk,
        spender_nonce,
        fee_amount,
        "invoke-simple",
        invoke_contract,
    );

    spender_nonce += 1;

    info!("Submit 2.05 txs");
    submit_tx(&http_origin, &publish_trait);
    submit_tx(&http_origin, &publish_impl);
    submit_tx(&http_origin, &publish_use);
    submit_tx(&http_origin, &publish_invoke);

    info!(
        "At height = {}, epoch-2.1 = {}",
        get_chain_info(&conf).burn_block_height,
        epoch_2_1
    );
    // wait until just before epoch 2.1
    loop {
        let tip_info = get_chain_info(&conf);
        if tip_info.burn_block_height >= epoch_2_1 - 3 {
            break;
        }
        next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);
    }

    // submit invocation txs.
    let tx_1 = make_contract_call(
        &spender_sk,
        spender_nonce,
        fee_amount,
        &contract_addr,
        "invoke-simple",
        "invocation-1",
        &[],
    );
    let expected_good_205_1_nonce = spender_nonce;
    spender_nonce += 1;

    let tx_2 = make_contract_call(
        &spender_sk,
        spender_nonce,
        fee_amount,
        &contract_addr,
        "invoke-simple",
        "invocation-2",
        &[Value::Principal(impl_contract_id.clone().into())],
    );
    let expected_good_205_2_nonce = spender_nonce;
    spender_nonce += 1;

    submit_tx(&http_origin, &tx_1);
    submit_tx(&http_origin, &tx_2);

    // this mines bitcoin block epoch_2_1 - 2, and causes the the
    // stacks node to mine the stacks block which will be included in
    // epoch_2_1 - 1, so these are the last transactions processed pre-2.1.
    next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);

    // submit invocation txs.
    let tx_1 = make_contract_call(
        &spender_sk,
        spender_nonce,
        fee_amount,
        &contract_addr,
        "invoke-simple",
        "invocation-1",
        &[],
    );
    let expected_good_21_1_nonce = spender_nonce;
    spender_nonce += 1;

    let tx_2 = make_contract_call(
        &spender_sk,
        spender_nonce,
        fee_amount,
        &contract_addr,
        "invoke-simple",
        "invocation-2",
        &[Value::Principal(impl_contract_id.clone().into())],
    );
    let expected_good_21_2_nonce = spender_nonce;
    spender_nonce += 1;

    submit_tx(&http_origin, &tx_1);
    submit_tx(&http_origin, &tx_2);

    // this mines those transactions into epoch 2.1
    // mine until just before epoch 2.2
    loop {
        let tip_info = get_chain_info(&conf);
        if tip_info.burn_block_height >= epoch_2_2 - 3 {
            break;
        }
        next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);
    }

    // submit invocation txs.
    let tx_1 = make_contract_call(
        &spender_sk,
        spender_nonce,
        fee_amount,
        &contract_addr,
        "invoke-simple",
        "invocation-1",
        &[],
    );
    let expected_good_21_3_nonce = spender_nonce;
    spender_nonce += 1;

    let tx_2 = make_contract_call(
        &spender_sk,
        spender_nonce,
        fee_amount,
        &contract_addr,
        "invoke-simple",
        "invocation-2",
        &[Value::Principal(impl_contract_id.clone().into())],
    );
    let expected_good_21_4_nonce = spender_nonce;
    spender_nonce += 1;

    submit_tx(&http_origin, &tx_1);
    submit_tx(&http_origin, &tx_2);

    // this mines bitcoin block epoch_2_2 - 2, and causes the the
    // stacks node to mine the stacks block which will be included in
    // epoch_2_2 - 1, so these are the last transactions processed pre-2.2.
    next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);

    let publish_invoke = make_contract_publish(
        &spender_sk,
        spender_nonce,
        fee_amount,
        "wrap-simple",
        wrapper_contract,
    );

    spender_nonce += 1;
    submit_tx(&http_origin, &publish_invoke);

    next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);
    next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);

    // submit invocation txs.
    let tx_1 = make_contract_call(
        &spender_sk,
        spender_nonce,
        fee_amount,
        &contract_addr,
        "wrap-simple",
        "invocation-1",
        &[],
    );
    let expected_bad_22_1_nonce = spender_nonce;
    spender_nonce += 1;

    let tx_2 = make_contract_call(
        &spender_sk,
        spender_nonce,
        fee_amount,
        &contract_addr,
        "wrap-simple",
        "invocation-2",
        &[Value::Principal(impl_contract_id.clone().into())],
    );
    let expected_bad_22_2_nonce = spender_nonce;
    spender_nonce += 1;

    submit_tx(&http_origin, &tx_1);
    submit_tx(&http_origin, &tx_2);

    // this mines those transactions into epoch 2.2
    // mine until just before epoch 2.3
    loop {
        let tip_info = get_chain_info(&conf);
        if tip_info.burn_block_height >= epoch_2_3 - 3 {
            break;
        }
        next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);
    }

    // submit invocation txs in epoch 2.2.
    let tx_1 = make_contract_call(
        &spender_sk,
        spender_nonce,
        fee_amount,
        &contract_addr,
        "wrap-simple",
        "invocation-1",
        &[],
    );
    let expected_bad_22_3_nonce = spender_nonce;
    spender_nonce += 1;

    let tx_2 = make_contract_call(
        &spender_sk,
        spender_nonce,
        fee_amount,
        &contract_addr,
        "wrap-simple",
        "invocation-2",
        &[Value::Principal(impl_contract_id.clone().into())],
    );
    let expected_bad_22_4_nonce = spender_nonce;
    spender_nonce += 1;

    submit_tx(&http_origin, &tx_1);
    submit_tx(&http_origin, &tx_2);

    // this mines bitcoin block epoch_2_3 - 2, and causes the the
    // stacks node to mine the stacks block which will be included in
    // epoch_2_3 - 1, so these are the last transactions processed pre-2.3.
    next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);

    let tx_3 = make_contract_call(
        &spender_sk,
        spender_nonce,
        fee_amount,
        &contract_addr,
        "wrap-simple",
        "invocation-1",
        &[],
    );
    let expected_good_23_3_nonce = spender_nonce;
    spender_nonce += 1;

    let tx_4 = make_contract_call(
        &spender_sk,
        spender_nonce,
        fee_amount,
        &contract_addr,
        "wrap-simple",
        "invocation-2",
        &[Value::Principal(impl_contract_id.clone().into())],
    );
    let expected_good_23_4_nonce = spender_nonce;
    spender_nonce += 1;

    submit_tx(&http_origin, &tx_3);
    submit_tx(&http_origin, &tx_4);

    // advance to epoch_2_3 before submitting the next transactions,
    //  so that they can pass the mempool.
    next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);
    next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);

    // submit invocation txs.
    let tx_1 = make_contract_call(
        &spender_sk,
        spender_nonce,
        fee_amount,
        &contract_addr,
        "invoke-simple",
        "invocation-1",
        &[],
    );
    let expected_good_23_1_nonce = spender_nonce;
    spender_nonce += 1;

    let tx_2 = make_contract_call(
        &spender_sk,
        spender_nonce,
        fee_amount,
        &contract_addr,
        "invoke-simple",
        "invocation-2",
        &[Value::Principal(impl_contract_id.clone().into())],
    );
    let expected_good_23_2_nonce = spender_nonce;
    spender_nonce += 1;

    submit_tx(&http_origin, &tx_1);
    submit_tx(&http_origin, &tx_2);

    next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);
    next_block_and_wait(&mut btc_regtest_controller, &blocks_processed);

    info!("Total spender txs = {}", spender_nonce);

    let blocks = test_observer::get_blocks();

    let mut transaction_receipts = Vec::new();

    for block in blocks {
        let transactions = block.get("transactions").unwrap().as_array().unwrap();
        for tx in transactions {
            let raw_tx = tx.get("raw_tx").unwrap().as_str().unwrap();
            if raw_tx == "0x00" {
                continue;
            }
            let tx_bytes = hex_bytes(&raw_tx[2..]).unwrap();
            let parsed =
                StacksTransaction::consensus_deserialize(&mut tx_bytes.as_slice()).unwrap();
            let tx_sender = PrincipalData::from(parsed.auth.origin().address_testnet());
            if &tx_sender == &spender_addr {
                let contract_call = match &parsed.payload {
                    TransactionPayload::ContractCall(cc) => cc,
                    // only interested in contract calls
                    _ => continue,
                };
                let result = Value::try_deserialize_hex_untyped(
                    tx.get("raw_result").unwrap().as_str().unwrap(),
                )
                .unwrap();

                transaction_receipts.push((
                    parsed.auth.get_origin_nonce(),
                    (contract_call.clone(), result),
                ));
            }
        }
    }

    transaction_receipts.sort_by_key(|x| x.0);

    let transaction_receipts: HashMap<_, _> = transaction_receipts.into_iter().collect();

    for tx_nonce in [
        expected_good_205_1_nonce,
        expected_good_21_1_nonce,
        expected_good_21_3_nonce,
        expected_good_23_1_nonce,
    ] {
        assert_eq!(
            transaction_receipts[&tx_nonce].0.contract_name.as_str(),
            "invoke-simple"
        );
        assert_eq!(
            transaction_receipts[&tx_nonce].0.function_name.as_str(),
            "invocation-1"
        );
        assert_eq!(&transaction_receipts[&tx_nonce].1.to_string(), "(ok u0)");
    }

    for tx_nonce in [
        expected_good_205_2_nonce,
        expected_good_21_2_nonce,
        expected_good_21_4_nonce,
        expected_good_23_2_nonce,
    ] {
        assert_eq!(
            transaction_receipts[&tx_nonce].0.contract_name.as_str(),
            "invoke-simple"
        );
        assert_eq!(
            transaction_receipts[&tx_nonce].0.function_name.as_str(),
            "invocation-2"
        );
        assert_eq!(&transaction_receipts[&tx_nonce].1.to_string(), "(ok u0)");
    }

    for tx_nonce in [expected_good_23_3_nonce] {
        assert_eq!(
            transaction_receipts[&tx_nonce].0.contract_name.as_str(),
            "wrap-simple"
        );
        assert_eq!(
            transaction_receipts[&tx_nonce].0.function_name.as_str(),
            "invocation-1"
        );
        assert_eq!(&transaction_receipts[&tx_nonce].1.to_string(), "(ok u0)");
    }

    for tx_nonce in [expected_good_23_4_nonce] {
        assert_eq!(
            transaction_receipts[&tx_nonce].0.contract_name.as_str(),
            "wrap-simple"
        );
        assert_eq!(
            transaction_receipts[&tx_nonce].0.function_name.as_str(),
            "invocation-2"
        );
        assert_eq!(&transaction_receipts[&tx_nonce].1.to_string(), "(ok u0)");
    }

    for tx_nonce in [expected_bad_22_1_nonce, expected_bad_22_3_nonce] {
        assert_eq!(
            transaction_receipts[&tx_nonce].0.contract_name.as_str(),
            "wrap-simple"
        );
        assert_eq!(
            transaction_receipts[&tx_nonce].0.function_name.as_str(),
            "invocation-1"
        );
        assert_eq!(&transaction_receipts[&tx_nonce].1.to_string(), "(err none)");
    }

    for tx_nonce in [expected_bad_22_2_nonce, expected_bad_22_4_nonce] {
        assert_eq!(
            transaction_receipts[&tx_nonce].0.contract_name.as_str(),
            "wrap-simple"
        );
        assert_eq!(
            transaction_receipts[&tx_nonce].0.function_name.as_str(),
            "invocation-2"
        );
        assert_eq!(&transaction_receipts[&tx_nonce].1.to_string(), "(err none)");
    }

    for (key, value) in transaction_receipts.iter() {
        eprintln!("{} => {} of {}", key, value.0, value.1);
    }

    test_observer::clear();
    channel.stop_chains_coordinator();
}
