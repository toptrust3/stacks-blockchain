use crate::vm::contexts::GlobalContext;
use crate::vm::costs::cost_functions::ClarityCostFunction;
use crate::vm::costs::{cost_functions, runtime_cost, CostTracker};
use crate::vm::errors::{
    check_argument_count, check_arguments_at_least, check_arguments_at_most, CheckErrors, Error,
    InterpreterError, InterpreterResult as Result, RuntimeErrorType,
};
use crate::vm::representations::ClarityName;
use crate::vm::representations::SymbolicExpression;
use crate::vm::types::{
    signatures::BUFF_1, signatures::BUFF_20, BuffData, BufferLength, CharType, OptionalData,
    PrincipalData, QualifiedContractIdentifier, ResponseData, SequenceData, SequenceSubtype,
    StandardPrincipalData, TupleData, TypeSignature, Value,
};
use crate::vm::{eval, ContractName, Environment, LocalContext};
use stacks_common::util::hash::hex_bytes;
use std::convert::TryFrom;

use crate::vm::database::ClarityDatabase;
use crate::vm::database::STXBalance;

use stacks_common::address::{
    C32_ADDRESS_VERSION_MAINNET_MULTISIG, C32_ADDRESS_VERSION_MAINNET_SINGLESIG,
    C32_ADDRESS_VERSION_TESTNET_MULTISIG, C32_ADDRESS_VERSION_TESTNET_SINGLESIG,
};

use crate::vm::ast::parser::{CONTRACT_MAX_NAME_LENGTH, CONTRACT_MIN_NAME_LENGTH};

/// Returns true if `version` indicates a mainnet address.
fn version_matches_mainnet(version: u8) -> bool {
    version == C32_ADDRESS_VERSION_MAINNET_MULTISIG
        || version == C32_ADDRESS_VERSION_MAINNET_SINGLESIG
}

/// Returns true if `version` indicates a testnet address.
fn version_matches_testnet(version: u8) -> bool {
    version == C32_ADDRESS_VERSION_TESTNET_MULTISIG
        || version == C32_ADDRESS_VERSION_TESTNET_SINGLESIG
}

/// Returns true if `version` indicates an address type that matches the network we are "currently
/// operating in", as indicated by the GlobalContext.
fn version_matches_current_network(version: u8, global_context: &GlobalContext) -> bool {
    let context_is_mainnet = global_context.mainnet;
    let context_is_testnet = !global_context.mainnet;

    // Note: It is possible for the version to match neither mainnet or testnet.
    (version_matches_mainnet(version) && context_is_mainnet)
        || (version_matches_testnet(version) && context_is_testnet)
}

pub fn special_is_standard(
    args: &[SymbolicExpression],
    env: &mut Environment,
    context: &LocalContext,
) -> Result<Value> {
    check_argument_count(1, args)?;
    runtime_cost(ClarityCostFunction::Unimplemented, env, 0)?;
    let owner = eval(&args[0], env, context)?;

    let version = match owner {
        Value::Principal(PrincipalData::Standard(StandardPrincipalData(version, _bytes))) => {
            version
        }
        Value::Principal(PrincipalData::Contract(QualifiedContractIdentifier {
            issuer,
            name: _,
        })) => issuer.0,
        _ => return Err(CheckErrors::TypeValueError(TypeSignature::PrincipalType, owner).into()),
    };

    Ok(Value::Bool(version_matches_current_network(
        version,
        env.global_context,
    )))
}

/// Creates a Tuple which is the result of parsing a Principal tuple into a Tuple of its `version`
/// and `hash-bytes`.
fn create_principal_parse_tuple(
    version: u8,
    hash_bytes: &[u8; 20],
    name_opt: Option<ContractName>,
) -> Value {
    Value::Tuple(
        TupleData::from_data(vec![
            (
                "version".into(),
                Value::Sequence(SequenceData::Buffer(BuffData {
                    data: vec![version],
                })),
            ),
            (
                "hash-bytes".into(),
                Value::Sequence(SequenceData::Buffer(BuffData {
                    data: hash_bytes.to_vec(),
                })),
            ),
            (
                "name".into(),
                Value::Optional(OptionalData {
                    data: name_opt.map(|name| Box::new(Value::from(name))),
                }),
            ),
        ])
        .expect("FAIL: Failed to initialize tuple."),
    )
}

/// Creates Response return type, to wrap an *actual error* result of a `principal-construct` or
/// `principal-parse`.
///
/// The response is an error Response, where the `err` value is a tuple `{error_int,parse_tuple}`.
/// `error_int` is of type `UInt`, `parse_tuple` is None.
fn create_principal_true_error_response(error_int: u32) -> Value {
    Value::Response(ResponseData {
        committed: false,
        data: Box::new(Value::Tuple(
            TupleData::from_data(vec![
                ("error_int".into(), Value::UInt(error_int.into())),
                ("value".into(), Value::none()),
            ])
            .expect("FAIL: Failed to initialize tuple."),
        )),
    })
}

/// Creates Response return type, to wrap a *return value returned as an error* result of a
/// `principal-construct` or `principal-parse`.
///
/// The response is an error Response, where the `err` value is a tuple `{error_int,value}`.
/// `error_int` is of type `UInt`, `value` is of type `Some(Value)`.
fn create_principal_value_error_response(error_int: u32, value: Value) -> Value {
    Value::Response(ResponseData {
        committed: false,
        data: Box::new(Value::Tuple(
            TupleData::from_data(vec![
                ("error_int".into(), Value::UInt(error_int.into())),
                (
                    "value".into(),
                    Value::some(value).expect("Unexpected problem creating Value."),
                ),
            ])
            .expect("FAIL: Failed to initialize tuple."),
        )),
    })
}

pub fn special_principal_parse(
    args: &[SymbolicExpression],
    env: &mut Environment,
    context: &LocalContext,
) -> Result<Value> {
    check_argument_count(1, args)?;
    runtime_cost(ClarityCostFunction::Unimplemented, env, 0)?;

    let principal = eval(&args[0], env, context)?;

    let (version_byte, hash_bytes, name_opt) = match principal {
        Value::Principal(PrincipalData::Standard(StandardPrincipalData(version, bytes))) => {
            (version, bytes, None)
        }
        Value::Principal(PrincipalData::Contract(QualifiedContractIdentifier { issuer, name })) => {
            (issuer.0, issuer.1, Some(name))
        }
        _ => {
            return Err(CheckErrors::TypeValueError(TypeSignature::PrincipalType, principal).into())
        }
    };

    // `version_byte_is_valid` determines whether the returned `Response` is through the success
    // channel or the error channel.
    let version_byte_is_valid = version_matches_current_network(version_byte, env.global_context);

    let tuple = create_principal_parse_tuple(version_byte, &hash_bytes, name_opt);
    Ok(Value::Response(ResponseData {
        committed: version_byte_is_valid,
        data: Box::new(tuple),
    }))
}

pub fn special_principal_construct(
    args: &[SymbolicExpression],
    env: &mut Environment,
    context: &LocalContext,
) -> Result<Value> {
    check_arguments_at_least(2, args)?;
    check_arguments_at_most(3, args)?;
    runtime_cost(ClarityCostFunction::Unimplemented, env, 0)?;

    let version = eval(&args[0], env, context)?;
    let hash_bytes = eval(&args[1], env, context)?;
    let name_opt = if args.len() > 2 {
        Some(eval(&args[2], env, context)?)
    } else {
        None
    };

    // Check the version byte.
    let verified_version = match version {
        Value::Sequence(SequenceData::Buffer(BuffData { ref data })) => data,
        _ => {
            return {
                // This is an aborting error because this should have been caught in analysis pass.
                Err(CheckErrors::TypeValueError(BUFF_1.clone(), version).into())
            };
        }
    };

    // This is an aborting error because this should have been caught in analysis pass.
    if verified_version.len() > 1 {
        return Err(CheckErrors::TypeValueError(BUFF_1.clone(), version).into());
    }

    // If the version byte buffer has 0 bytes, this is a recoverable error, because it wasn't the
    // job of the type system.
    if verified_version.len() < 1 {
        // do some kind of error
        return Ok(create_principal_true_error_response(1));
    }

    // Assume: verified_version.len() == 1
    let version_byte = (*verified_version)[0];

    // If the version byte is >= 32, this is a runtime error, because it wasn't the job of the
    // type system.  This is a requirement for c32check encoding.
    if version_byte >= 32 {
        return Ok(create_principal_true_error_response(1));
    }

    // `version_byte_is_valid` determines whether the returned `Response` is through the success
    // channel or the error channel.
    let version_byte_is_valid = version_matches_current_network(version_byte, env.global_context);

    // Check the hash bytes -- they must be a (buff 20).
    // This is an aborting error because this should have been caught in analysis pass.
    let verified_hash_bytes = match hash_bytes {
        Value::Sequence(SequenceData::Buffer(BuffData { ref data })) => data,
        _ => return Err(CheckErrors::TypeValueError(BUFF_20.clone(), hash_bytes).into()),
    };

    // This must have been a (buff 20).
    // This is an aborting error because this should have been caught in analysis pass.
    if verified_hash_bytes.len() > 20 {
        return Err(CheckErrors::TypeValueError(BUFF_20.clone(), hash_bytes).into());
    }

    // If the hash-bytes buffer has less than 20 bytes, this is a runtime error, because it
    // wasn't the job of the type system (i.e. (buff X) for all X < 20 are all also (buff 20))
    if verified_hash_bytes.len() < 20 {
        return Ok(create_principal_true_error_response(1));
    }

    // Construct the principal.
    let mut transfer_buffer = [0u8; 20];
    transfer_buffer.copy_from_slice(&verified_hash_bytes);
    let principal_data = StandardPrincipalData(version_byte, transfer_buffer);

    let principal = if let Some(name) = name_opt {
        // requested a contract principal.  Verify that the `name` is a valid ContractName.
        // The type-checker will have verified that it's (string-ascii 40), but not long enough.
        let name_bytes = match name {
            Value::Sequence(SequenceData::String(CharType::ASCII(ref ascii_data))) => {
                &ascii_data.data
            }
            _ => {
                return Err(CheckErrors::TypeValueError(
                    TypeSignature::contract_name_string_ascii(),
                    name,
                )
                .into())
            }
        };

        // If it's not long enough, then it's a runtime error that warrants an (err ..) response.
        if name_bytes.len() < CONTRACT_MIN_NAME_LENGTH {
            return Ok(create_principal_true_error_response(2));
        }

        // if it's too long, then this should have been caught by the type-checker
        if name_bytes.len() > CONTRACT_MAX_NAME_LENGTH {
            return Err(CheckErrors::TypeValueError(
                TypeSignature::contract_name_string_ascii(),
                name,
            )
            .into());
        }

        // The type-checker can't verify that the name is a valid ContractName, so we'll need to do
        // it here at runtime.  If it's not valid, then it warrants this function evaluating to
        // (err ..).
        let name_string = match name {
            // destruct again to avoid a .clone() on the inner ascii_data.data
            Value::Sequence(SequenceData::String(CharType::ASCII(ascii_data))) => {
                String::from_utf8(ascii_data.data).expect("FAIL: could not convert bytes of type (string-ascii 40) back to a UTF-8 string")
            },
            _ => {
                unreachable!()
            }
        };

        let contract_name = match ContractName::try_from(name_string) {
            Ok(cn) => cn,
            Err(_) => {
                // not a valid contract name
                return Ok(create_principal_true_error_response(2));
            }
        };

        Value::Principal(PrincipalData::Contract(QualifiedContractIdentifier::new(
            principal_data,
            contract_name,
        )))
    } else {
        // requested a standard principal
        Value::Principal(PrincipalData::Standard(principal_data))
    };

    if version_byte_is_valid {
        Ok(Value::Response(ResponseData {
            committed: version_byte_is_valid,
            data: Box::new(principal),
        }))
    } else {
        Ok(create_principal_value_error_response(0, principal))
    }
}
