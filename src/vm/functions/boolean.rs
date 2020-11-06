// Copyright (C) 2013-2020 Blocstack PBC, a public benefit corporation
// Copyright (C) 2020 Stacks Open Internet Foundation
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

use vm::costs::{cost_functions, runtime_cost};
use vm::errors::{
    check_argument_count, check_arguments_at_least, CheckErrors, InterpreterResult as Result,
};
use vm::representations::SymbolicExpression;
use vm::types::{TypeSignature, Value};
use vm::{eval, Environment, LocalContext};
use vm::costs::cost_functions::ClarityCostFunction;

fn type_force_bool(value: &Value) -> Result<bool> {
    match *value {
        Value::Bool(boolean) => Ok(boolean),
        _ => Err(CheckErrors::TypeValueError(TypeSignature::BoolType, value.clone()).into()),
    }
}

pub fn special_or(
    args: &[SymbolicExpression],
    env: &mut Environment,
    context: &LocalContext,
) -> Result<Value> {
    check_arguments_at_least(1, args)?;

    runtime_cost(ClarityCostFunction::Or, env, args.len())?;

    for arg in args.iter() {
        let evaluated = eval(&arg, env, context)?;
        let result = type_force_bool(&evaluated)?;
        if result {
            return Ok(Value::Bool(true));
        }
    }

    Ok(Value::Bool(false))
}

pub fn special_and(
    args: &[SymbolicExpression],
    env: &mut Environment,
    context: &LocalContext,
) -> Result<Value> {
    check_arguments_at_least(1, args)?;

    runtime_cost(ClarityCostFunction::And, env, args.len())?;

    for arg in args.iter() {
        let evaluated = eval(&arg, env, context)?;
        let result = type_force_bool(&evaluated)?;
        if !result {
            return Ok(Value::Bool(false));
        }
    }

    Ok(Value::Bool(true))
}

pub fn native_not(input: Value) -> Result<Value> {
    let value = type_force_bool(&input)?;
    Ok(Value::Bool(!value))
}
