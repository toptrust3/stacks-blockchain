use super::super::types::{ValueType, DefinedFunction};
use super::super::representations::SymbolicExpression;
use super::super::representations::SymbolicExpression::{Atom,AtomValue,List,TypeIdentifier};
use super::super::{Context,Environment,eval};
use super::super::errors::Error;

pub enum DefineResult {
    Variable(String, ValueType),
    Function(String, DefinedFunction),
    NoDefine
}

pub fn handle_define_variable(variable: &String, expression: &SymbolicExpression, env: &mut Environment) -> Result<DefineResult, Error> {
    let context = Context::new();
    let value = eval(expression, env, &context)?;
    Ok(DefineResult::Variable(variable.clone(), value))
}

pub fn handle_define_function(signature: &[SymbolicExpression], expression: &SymbolicExpression) -> Result<DefineResult, Error> {
    let coerced_atoms: Result<Vec<_>, _> = signature.iter().map(|x| {
        if let Atom(name) = x {
            Ok(name)
        } else {
            Err(Error::InvalidArguments("Non-atomic argument to method signature in define".to_string()))
        }
    }).collect();

    let names = coerced_atoms?;
    if let Some((function_name, arg_names)) = names.split_first() {
        let function = DefinedFunction {
            arguments: arg_names.iter().map(|x| (*x).clone()).collect(),
            body: expression.clone()
        };
        Ok(DefineResult::Function((*function_name).clone(), function))
    } else {
        Err(Error::InvalidArguments("Must supply atleast a name argument to define a function".to_string()))
    }
}

pub fn evaluate_define(expression: &SymbolicExpression, env: &mut Environment) -> Result<DefineResult, Error> {
    if let SymbolicExpression::List(elements) = expression {
        if elements.len() != 3 || elements[0] != Atom("define".to_string()) {
            Ok(DefineResult::NoDefine)
        } else {
            match elements[1] {
                Atom(ref variable) => handle_define_variable(variable, &elements[2], env),
                AtomValue(ref _value) => Err(Error::InvalidArguments(
                    "Illegal operation: attempted to re-define a value type.".to_string())),
                TypeIdentifier(ref _value) => Err(Error::InvalidArguments(
                    "Illegal operation: attempted to re-define a type identifier.".to_string())),
                List(ref function_signature) => handle_define_function(&function_signature, &elements[2])
            }
        }
    } else {
        Ok(DefineResult::NoDefine)
    }
}
