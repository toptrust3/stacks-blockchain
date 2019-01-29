use super::InterpreterResult;
use super::super::errors::Error;
use super::super::types::{ValueType, TupleData};
use super::super::representations::SymbolicExpression;
use super::super::representations::SymbolicExpression::{NamedParameter};
use super::super::{Context,Environment,eval};

pub fn tuple_cons(args: &[SymbolicExpression], env: &mut Environment, context: &Context) -> InterpreterResult {
    // (tuple #arg-name value
    //        #arg-name value ...)
    if args.len() % 2 != 0 {
        return Err(Error::InvalidArguments(format!("Tuples must be constructed with named-arguments and corresponding values")))
    }
    let num_pairs = args.len() / 2;
    // turn list into pairs.
    let eval_result: Result<Vec<_>, Error> = (0..num_pairs).map(|i| {
        let arg_name = match args[i * 2] {
            NamedParameter(ref name) => Ok(name.as_str()),
            _ => Err(Error::InvalidArguments("Named arguments must be supplied as #name-arg".to_string()))
        }?;
        let value = eval(&args[i * 2 + 1], env, context)?;
        Ok((arg_name, value))
    }).collect();

    let evaled_pairs = eval_result?;

    let tuple_data = TupleData::from_data(&evaled_pairs)?;
    Ok(ValueType::TupleType(tuple_data))
}

pub fn tuple_get(args: &[SymbolicExpression], env: &mut Environment, context: &Context) -> InterpreterResult {
    // (get arg-name (tuple ...))
    //    if the tuple argument is 'null, then return 'null.
    //  NOTE:  a tuple field value itself may _never_ be 'null

    if args.len() != 2 {
        return Err(Error::InvalidArguments(format!("(get ..) requires exactly 2 arguments")))
    }
    let arg_name = match args[0] {
        SymbolicExpression::Atom(ref name) => Ok(name),
        _ => Err(Error::InvalidArguments(format!("Second argument to (get ..) must be a name, found: {:?}", args[0])))
    }?;

    let value = eval(&args[1], env, context)?;

    match value {
        ValueType::VoidType => Ok(ValueType::VoidType),
        ValueType::TupleType(tuple_data) => tuple_data.get(arg_name),
        _ => Err(Error::TypeError("TupleType".to_string(), value.clone()))
    }
}
