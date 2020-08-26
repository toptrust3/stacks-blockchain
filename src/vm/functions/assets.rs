use vm::functions::tuples;
use vm::functions::tuples::TupleDefinitionType::{Implicit, Explicit};

use vm::types::{Value, OptionalData, BuffData, PrincipalData, BlockInfoProperty, TypeSignature, AssetIdentifier};
use vm::representations::{SymbolicExpression};
use vm::errors::{Error, InterpreterError, CheckErrors, RuntimeErrorType, InterpreterResult as Result, check_argument_count};
use vm::{eval, LocalContext, Environment};
use vm::costs::{cost_functions, CostTracker};
use std::convert::{TryFrom};

use vm::database::ClarityDatabase;

enum MintAssetErrorCodes { ALREADY_EXIST = 1 }
enum MintTokenErrorCodes { NON_POSITIVE_AMOUNT = 1 }
enum TransferAssetErrorCodes { NOT_OWNED_BY = 1, SENDER_IS_RECIPIENT = 2, DOES_NOT_EXIST = 3 }
enum TransferTokenErrorCodes { NOT_ENOUGH_BALANCE = 1, SENDER_IS_RECIPIENT = 2, NON_POSITIVE_AMOUNT = 3 }
enum StxErrorCodes { NOT_ENOUGH_BALANCE = 1, SENDER_IS_RECIPIENT = 2, NON_POSITIVE_AMOUNT = 3, SENDER_IS_NOT_TX_SENDER = 4 }

macro_rules! clarity_ecode {
    ($thing:expr) => {
        Ok(Value::err_uint($thing as u128))
    }
}

/// Determine the "true" balance, given an unlock height and current burn block height
pub fn stx_balance_with_unlock(balance_raw: u128, stx_locked: u128, unlock_height: u64, cur_burn_height: u64) -> (u128, bool) {
    let (balance, consolidated) = 
        if unlock_height != 0 && unlock_height <= cur_burn_height {
            (balance_raw.checked_add(stx_locked).expect("STX overflow"), true)
        }
        else {
            (balance_raw, false)
        };

    (balance, consolidated)
}

/// Get the consolidated uSTX balance.
/// That is, if the PoX lock has expired, then include the
/// no-longer-locked uSTX in the balance.
/// Returns (balance, is-consolidated?)
pub fn stx_balance_consolidated(db: &mut ClarityDatabase, principal: &PrincipalData) -> (u128, bool) {
    let balance_raw         = db.get_account_stx_balance(principal);
    let stx_locked          = db.get_account_stx_locked(principal);
    let unlock_height       = db.get_account_unlock_height(principal);
    let cur_burn_height     = db.get_current_burnchain_block_height() as u64;

    let (total_balance, consolidated) = stx_balance_with_unlock(balance_raw, stx_locked, unlock_height, cur_burn_height);
    test_debug!("Balance of {} (raw={},locked={},unlock-height={},current-height={}) is {} (consolidated={})", principal, balance_raw, stx_locked, unlock_height, cur_burn_height, total_balance, consolidated);
    (total_balance, consolidated)
}

pub fn special_stx_balance(args: &[SymbolicExpression],
                            env: &mut Environment,
                            context: &LocalContext) -> Result<Value> {
    check_argument_count(1, args)?;

    runtime_cost!(cost_functions::STX_BALANCE, env, 0)?;

    let owner = eval(&args[0], env, context)?;

    if let Value::Principal(ref principal) = owner {
        let (total_balance, _) = stx_balance_consolidated(&mut env.global_context.database, principal);
        Ok(Value::UInt(total_balance))
    } else {
        Err(CheckErrors::TypeValueError(TypeSignature::PrincipalType, owner).into())
    }
}

/// Do a "consolidated" STX transfer.
/// If the 'from' principal has locked STX, and they have unlocked, then process the STX unlock
/// and update its balance in addition to spending tokens out of it.
pub fn stx_transfer_consolidated(env: &mut Environment, from: &PrincipalData, to: &PrincipalData, amount: u128) -> Result<Value> {
    if amount <= 0 {
        return clarity_ecode!(StxErrorCodes::NON_POSITIVE_AMOUNT);
    }

    if from == to {
        return clarity_ecode!(StxErrorCodes::SENDER_IS_RECIPIENT);
    }

    if Some(from.clone()) != env.sender.as_ref().map(|pval| pval.clone().expect_principal()) {
        return clarity_ecode!(StxErrorCodes::SENDER_IS_NOT_TX_SENDER);
    }

    let (from_bal, unlock) = stx_balance_consolidated(&mut env.global_context.database, from);
    let to_bal = env.global_context.database.get_account_stx_balance(to);

    if from_bal < amount {
        return clarity_ecode!(StxErrorCodes::NOT_ENOUGH_BALANCE)
    }

    let final_from_bal = from_bal - amount;
    let final_to_bal = to_bal.checked_add(amount)
        .ok_or(RuntimeErrorType::ArithmeticOverflow)?;

    // loading from/to principals and balances
    env.add_memory(TypeSignature::PrincipalType.size() as u64)?;
    env.add_memory(TypeSignature::PrincipalType.size() as u64)?;
    env.add_memory(TypeSignature::UIntType.size() as u64)?;
    env.add_memory(TypeSignature::UIntType.size() as u64)?;

    // loading from's locked amount and height
    // TODO: this does not count the inner stacks block header load, but arguably,
    // this could be optimized away, so it shouldn't penalize the caller.
    env.add_memory(TypeSignature::UIntType.size() as u64)?;
    env.add_memory(TypeSignature::UIntType.size() as u64)?;

    // NOTE: this updates the balance with the unlocked tokens, if we did an unlock.
    env.global_context.database.set_account_stx_balance(from, final_from_bal);
    env.global_context.database.set_account_stx_balance(to,   final_to_bal);

    if unlock {
        // clear lock state
        env.global_context.database.set_account_stx_locked(from, 0);
        env.global_context.database.set_account_unlock_height(from, 0);
        debug!("Consolidated {} after stx-transfer", from);
    }

    env.global_context.log_stx_transfer(&from, amount)?;
    env.register_stx_transfer_event(from.clone(), to.clone(), amount)?;
    Ok(Value::okay_true())
}

pub fn special_stx_transfer(args: &[SymbolicExpression],
                            env: &mut Environment,
                            context: &LocalContext) -> Result<Value> {
    check_argument_count(3, args)?;

    runtime_cost!(cost_functions::STX_TRANSFER, env, 0)?;

    let amount_val = eval(&args[0], env, context)?;
    let from_val   = eval(&args[1], env, context)?;
    let to_val     = eval(&args[2], env, context)?;

    if let (Value::Principal(ref from), Value::Principal(ref to), Value::UInt(amount)) = (&from_val, to_val, amount_val) {
        stx_transfer_consolidated(env, from, to, amount)
    }
    else {
        Err(CheckErrors::BadTransferSTXArguments.into())
    }
}

pub fn special_stx_burn(args: &[SymbolicExpression],
                        env: &mut Environment,
                        context: &LocalContext) -> Result<Value> {
    check_argument_count(2, args)?;

    runtime_cost!(cost_functions::STX_TRANSFER, env, 0)?;

    let amount_val = eval(&args[0], env, context)?;
    let from_val   = eval(&args[1], env, context)?;

    if let (Value::Principal(ref from), Value::UInt(amount)) = (&from_val, amount_val) {
        if amount <= 0 {
            return clarity_ecode!(StxErrorCodes::NON_POSITIVE_AMOUNT)
        }

        if Some(&from_val) != env.sender.as_ref() {
            return clarity_ecode!(StxErrorCodes::SENDER_IS_NOT_TX_SENDER)
        }

        let (from_bal, unlock) = stx_balance_consolidated(&mut env.global_context.database, from);
        
        if from_bal < amount {
            return clarity_ecode!(StxErrorCodes::NOT_ENOUGH_BALANCE)
        }

        let final_from_bal = from_bal - amount;

        env.add_memory(TypeSignature::PrincipalType.size() as u64)?;
        env.add_memory(TypeSignature::UIntType.size() as u64)?;

        // for unlock-handling
        env.add_memory(TypeSignature::UIntType.size() as u64)?;
        env.add_memory(TypeSignature::UIntType.size() as u64)?;

        env.global_context.database.set_account_stx_balance(from, final_from_bal);

        if unlock {
            // clear lock state
            env.global_context.database.set_account_stx_locked(from, 0);
            env.global_context.database.set_account_unlock_height(from, 0);
            debug!("Consolidated {} after burn", from);
        }

        env.global_context.log_stx_burn(&from, amount)?;
        env.register_stx_burn_event(from.clone(), amount)?;

        Ok(Value::okay_true())

    } else {
        Err(CheckErrors::BadTransferSTXArguments.into())
    }
}

pub fn special_mint_token(args: &[SymbolicExpression],
                          env: &mut Environment,
                          context: &LocalContext) -> Result<Value> {
    check_argument_count(3, args)?;

    runtime_cost!(cost_functions::FT_MINT, env, 0)?;

    let token_name = args[0].match_atom()
        .ok_or(CheckErrors::BadTokenName)?;

    let amount = eval(&args[1], env, context)?;
    let to =     eval(&args[2], env, context)?;

    if let (Value::UInt(amount),
            Value::Principal(ref to_principal)) = (amount, to) {
        if amount <= 0 {
            return clarity_ecode!(MintTokenErrorCodes::NON_POSITIVE_AMOUNT);
        }

        env.global_context.database.checked_increase_token_supply(
            &env.contract_context.contract_identifier, token_name, amount)?;

        let to_bal = env.global_context.database.get_ft_balance(&env.contract_context.contract_identifier, token_name, to_principal)?;

        let final_to_bal = to_bal.checked_add(amount)
            .ok_or(RuntimeErrorType::ArithmeticOverflow)?;

        env.add_memory(TypeSignature::PrincipalType.size() as u64)?;
        env.add_memory(TypeSignature::UIntType.size() as u64)?;

        env.global_context.database.set_ft_balance(&env.contract_context.contract_identifier, token_name, to_principal, final_to_bal)?;

        let asset_identifier = AssetIdentifier {
            contract_identifier: env.contract_context.contract_identifier.clone(),
            asset_name: token_name.clone()
        };
        env.register_ft_mint_event(to_principal.clone(), amount, asset_identifier)?;

        Ok(Value::okay_true())
    } else {
        Err(CheckErrors::BadMintFTArguments.into())
    }
}

pub fn special_mint_asset(args: &[SymbolicExpression],
                          env: &mut Environment,
                          context: &LocalContext) -> Result<Value> {
    check_argument_count(3, args)?;

    let asset_name = args[0].match_atom()
        .ok_or(CheckErrors::BadTokenName)?;

    let asset =  eval(&args[1], env, context)?;
    let to    =  eval(&args[2], env, context)?;

    let expected_asset_type = env.global_context.database.get_nft_key_type(&env.contract_context.contract_identifier, asset_name)?;

    runtime_cost!(cost_functions::NFT_MINT, env, expected_asset_type.size())?;

    if !expected_asset_type.admits(&asset) {
        return Err(CheckErrors::TypeValueError(expected_asset_type, asset).into())
    }

    if let Value::Principal(ref to_principal) = to {
        match env.global_context.database.get_nft_owner(&env.contract_context.contract_identifier, asset_name, &asset) {
            Err(Error::Runtime(RuntimeErrorType::NoSuchToken, _)) => Ok(()),
            Ok(_owner) => return clarity_ecode!(MintAssetErrorCodes::ALREADY_EXIST),
            Err(e) => Err(e)
        }?;

        env.add_memory(TypeSignature::PrincipalType.size() as u64)?;
        env.add_memory(expected_asset_type.size() as u64)?;

        env.global_context.database.set_nft_owner(&env.contract_context.contract_identifier, asset_name, &asset, to_principal)?;

        let asset_identifier = AssetIdentifier {
            contract_identifier: env.contract_context.contract_identifier.clone(),
            asset_name: asset_name.clone()
        };
        env.register_nft_mint_event(to_principal.clone(), asset, asset_identifier)?;

        Ok(Value::okay_true())
    } else {
        Err(CheckErrors::TypeValueError(TypeSignature::PrincipalType, to).into())
    }
}

pub fn special_transfer_asset(args: &[SymbolicExpression],
                              env: &mut Environment,
                              context: &LocalContext) -> Result<Value> {
    check_argument_count(4, args)?;

    let asset_name = args[0].match_atom()
        .ok_or(CheckErrors::BadTokenName)?;

    let asset =  eval(&args[1], env, context)?;
    let from  =  eval(&args[2], env, context)?;
    let to    =  eval(&args[3], env, context)?;

    let expected_asset_type = env.global_context.database.get_nft_key_type(&env.contract_context.contract_identifier, asset_name)?;

    runtime_cost!(cost_functions::NFT_TRANSFER, env, expected_asset_type.size())?;

    if !expected_asset_type.admits(&asset) {
        return Err(CheckErrors::TypeValueError(expected_asset_type, asset).into())
    }

    if let (Value::Principal(ref from_principal),
            Value::Principal(ref to_principal)) = (from, to) {

        if from_principal == to_principal {
            return clarity_ecode!(TransferAssetErrorCodes::SENDER_IS_RECIPIENT)
        }

        let current_owner = match env.global_context.database.get_nft_owner(&env.contract_context.contract_identifier, asset_name, &asset) {
            Ok(owner) => Ok(owner),
            Err(Error::Runtime(RuntimeErrorType::NoSuchToken, _)) => {
                return clarity_ecode!(TransferAssetErrorCodes::DOES_NOT_EXIST)
            },
            Err(e) => Err(e)
        }?;
            

        if current_owner != *from_principal {
            return clarity_ecode!(TransferAssetErrorCodes::NOT_OWNED_BY)
        }

        env.add_memory(TypeSignature::PrincipalType.size() as u64)?;
        env.add_memory(expected_asset_type.size() as u64)?;

        env.global_context.database.set_nft_owner(&env.contract_context.contract_identifier, asset_name, &asset, to_principal)?;

        env.global_context.log_asset_transfer(from_principal, &env.contract_context.contract_identifier, asset_name, asset.clone());

        let asset_identifier = AssetIdentifier {
            contract_identifier: env.contract_context.contract_identifier.clone(),
            asset_name: asset_name.clone()
        };
        env.register_nft_transfer_event(from_principal.clone(), to_principal.clone(), asset, asset_identifier)?;

        Ok(Value::okay_true())
    } else {
        Err(CheckErrors::BadTransferNFTArguments.into())
    }
}

pub fn special_transfer_token(args: &[SymbolicExpression],
                              env: &mut Environment,
                              context: &LocalContext) -> Result<Value> {
    check_argument_count(4, args)?;

    runtime_cost!(cost_functions::FT_TRANSFER, env, 0)?;

    let token_name = args[0].match_atom()
        .ok_or(CheckErrors::BadTokenName)?;

    let amount = eval(&args[1], env, context)?;
    let from =   eval(&args[2], env, context)?;
    let to =     eval(&args[3], env, context)?;

    if let (Value::UInt(amount),
            Value::Principal(ref from_principal),
            Value::Principal(ref to_principal)) = (amount, from, to) {
        if amount <= 0 {
            return clarity_ecode!(TransferTokenErrorCodes::NON_POSITIVE_AMOUNT)
        }

        if from_principal == to_principal {
            return clarity_ecode!(TransferTokenErrorCodes::SENDER_IS_RECIPIENT)
        }

        let from_bal = env.global_context.database.get_ft_balance(&env.contract_context.contract_identifier, token_name, from_principal)?;

        if from_bal < amount {
            return clarity_ecode!(TransferTokenErrorCodes::NOT_ENOUGH_BALANCE)
        }

        let final_from_bal = from_bal - amount;

        let to_bal = env.global_context.database.get_ft_balance(&env.contract_context.contract_identifier, token_name, to_principal)?;

        let final_to_bal = to_bal.checked_add(amount)
            .ok_or(RuntimeErrorType::ArithmeticOverflow)?;

        env.add_memory(TypeSignature::PrincipalType.size() as u64)?;
        env.add_memory(TypeSignature::PrincipalType.size() as u64)?;
        env.add_memory(TypeSignature::UIntType.size() as u64)?;
        env.add_memory(TypeSignature::UIntType.size() as u64)?;

        env.global_context.database.set_ft_balance(&env.contract_context.contract_identifier, token_name, from_principal, final_from_bal)?;
        env.global_context.database.set_ft_balance(&env.contract_context.contract_identifier, token_name, to_principal, final_to_bal)?;

        env.global_context.log_token_transfer(from_principal, &env.contract_context.contract_identifier, token_name, amount)?;

        let asset_identifier = AssetIdentifier {
            contract_identifier: env.contract_context.contract_identifier.clone(),
            asset_name: token_name.clone()
        };
        env.register_ft_transfer_event(from_principal.clone(), to_principal.clone(), amount, asset_identifier)?;

        Ok(Value::okay_true())
    } else {
        Err(CheckErrors::BadTransferFTArguments.into())
    }
}

pub fn special_get_balance(args: &[SymbolicExpression],
                           env: &mut Environment,
                           context: &LocalContext) -> Result<Value> {
    check_argument_count(2, args)?;

    runtime_cost!(cost_functions::FT_BALANCE, env, 0)?;

    let token_name = args[0].match_atom()
        .ok_or(CheckErrors::BadTokenName)?;

    let owner = eval(&args[1], env, context)?;

    if let Value::Principal(ref principal) = owner {
        let balance = env.global_context.database.get_ft_balance(&env.contract_context.contract_identifier, token_name, principal)?;
        Ok(Value::UInt(balance))
    } else {
        Err(CheckErrors::TypeValueError(TypeSignature::PrincipalType, owner).into())
    }

}

pub fn special_get_owner(args: &[SymbolicExpression],
                         env: &mut Environment,
                         context: &LocalContext) -> Result<Value> {
    check_argument_count(2, args)?;

    let asset_name = args[0].match_atom()
        .ok_or(CheckErrors::BadTokenName)?;

    let asset = eval(&args[1], env, context)?;
    let expected_asset_type = env.global_context.database.get_nft_key_type(&env.contract_context.contract_identifier, asset_name)?;

    runtime_cost!(cost_functions::NFT_OWNER, env, expected_asset_type.size())?;

    if !expected_asset_type.admits(&asset) {
        return Err(CheckErrors::TypeValueError(expected_asset_type, asset).into())
    }

    match env.global_context.database.get_nft_owner(&env.contract_context.contract_identifier, asset_name, &asset) {
        Ok(owner) => Ok(Value::some(Value::Principal(owner))
                        .expect("Principal should always fit in optional.")),
        Err(Error::Runtime(RuntimeErrorType::NoSuchToken, _)) => Ok(Value::none()),
        Err(e) => Err(e)
    }
}
