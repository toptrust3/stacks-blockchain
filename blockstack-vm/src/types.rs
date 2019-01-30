use std::collections::BTreeMap;

use InterpreterResult;
use errors::{Error, InterpreterResult as Result};
use representations::SymbolicExpression;
use {Context,Environment};
use eval;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AtomTypeIdentifier {
    VoidType,
    IntType,
    BoolType,
    BufferType,
    TupleType(TupleTypeSignature)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeSignature {
    atomic_type: AtomTypeIdentifier,
    list_dimensions: Option<(u8, u8)>,
    // NOTE: for the purposes of type-checks and cost computations, list size = dimension * max_length!
    //       high dimensional lists are _expensive_ --- use lists of tuples!
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TupleTypeSignature {
    type_map: BTreeMap<String, TypeSignature>
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TupleData {
    pub type_signature: TupleTypeSignature,
    data_map: BTreeMap<String, Value>
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Value {
    Void,
    Int(i128),
    Bool(bool),
    Buffer(Box<[char]>),
    List(Vec<Value>, TypeSignature),
    Tuple(TupleData)
}

pub enum CallableType <'a> {
    UserFunction(Box<DefinedFunction>),
    NativeFunction(&'a Fn(&[Value]) -> InterpreterResult),
    SpecialFunction(&'a Fn(&[SymbolicExpression], &mut Environment, &Context) -> InterpreterResult)
}

#[derive(Clone)]
pub struct DefinedFunction {
    pub arguments: Vec<String>,
    pub body: SymbolicExpression
}

#[derive(Clone,PartialEq,Eq,Hash)]
pub struct FunctionIdentifier {
    pub arguments: Vec<String>,
    pub body: SymbolicExpression
}

impl TupleTypeSignature {
    pub fn new(type_data: Vec<(String, TypeSignature)>) -> Result<TupleTypeSignature> {
        let mut type_map = BTreeMap::new();
        for (name, type_info) in type_data {
            if let Some(_v) = type_map.insert(name, type_info) {
                return Err(Error::InvalidArguments("Cannot use named argument twice in tuple construction.".to_string()))
            }
        }
        Ok(TupleTypeSignature { type_map: type_map })
    }

    pub fn check_valid(&self, name: &str, value: &Value) -> bool {
        if let Some(expected_type) = self.type_map.get(name) {
            *expected_type == TypeSignature::type_of(value)
        } else {
            false
        }
    }
}

impl TupleData {
    pub fn from_data(data: &[(&str, Value)]) -> Result<TupleData> {
        let mut type_map = BTreeMap::new();
        let mut data_map = BTreeMap::new();
        for (name, value) in data {
            let type_info = TypeSignature::type_of(value);
            if type_info.atomic_type == AtomTypeIdentifier::VoidType {
                return Err(Error::InvalidArguments("Cannot use VoidTypes in tuples.".to_string()))
            }
            if let Some(_v) = type_map.insert(name.to_string(), type_info) {
                return Err(Error::InvalidArguments("Cannot use named argument twice in tuple construction.".to_string()))
            }
            data_map.insert(name.to_string(), (*value).clone());
        }
        Ok(TupleData { type_signature: TupleTypeSignature { type_map: type_map },
                       data_map: data_map })

    }

    pub fn get(&self, name: &str) -> InterpreterResult {
        if let Some(value) = self.data_map.get(name) {
            Ok(value.clone())
        } else {
            Err(Error::InvalidArguments(format!("No such field {:?} in tuple", name)))
        }
        
    }
}

impl TypeSignature {
    pub fn new_atom(atomic_type: AtomTypeIdentifier) -> TypeSignature {
        TypeSignature { atomic_type: atomic_type,
                        list_dimensions: None }
    }

    pub fn new_list(atomic_type: AtomTypeIdentifier, max_len: u8, dimension: u8) -> Result<TypeSignature> {
        if dimension == 0 {
            return Err(Error::InvalidArguments("Cannot construct list of dimension 0".to_string()))
        } else {
            Ok(TypeSignature { atomic_type: atomic_type,
                               list_dimensions: Some((max_len, dimension)) })
        }
    }

    pub fn get_empty_list_type() -> TypeSignature {
        TypeSignature { atomic_type: AtomTypeIdentifier::IntType,
                        list_dimensions: Some((0, 1)) }
    }

    pub fn type_of(x: &Value) -> TypeSignature {
        match x {
            Value::Void => TypeSignature::new_atom(AtomTypeIdentifier::VoidType),
            Value::Int(_v) => TypeSignature::new_atom(AtomTypeIdentifier::IntType),
            Value::Bool(_v) => TypeSignature::new_atom(AtomTypeIdentifier::BoolType),
            Value::Buffer(_v) => TypeSignature::new_atom(AtomTypeIdentifier::BufferType),
            Value::List(_v, type_signature) => type_signature.clone(),
            Value::Tuple(v) => TypeSignature::new_atom(AtomTypeIdentifier::TupleType(
                v.type_signature.clone()))
        }
    }

    pub fn get_list_type_for(x: &Value, max_len: u8) -> Result<TypeSignature> {
        match x {
            Value::Void => Err(Error::InvalidArguments("Cannot construct list of void types".to_string())),
            Value::Tuple(_a) => Err(Error::InvalidArguments("Cannot construct list of tuple types".to_string())),
            _ => {
                let mut base_type = TypeSignature::type_of(x);
                if let Some((child_max_len, dimension)) = base_type.list_dimensions {
                    if child_max_len > max_len {
                        base_type.list_dimensions = Some((child_max_len, dimension + 1));
                    } else {
                        base_type.list_dimensions = Some((max_len, dimension + 1));
                    }
                } else {
                    base_type.list_dimensions = Some((max_len, 1));
                }
                Ok(base_type)
            }
        }
    }

    pub fn construct_parent_list_type(args: &[Value]) -> Result<TypeSignature> {
        if let Some((first, rest)) = args.split_first() {
            // children must be all of identical types, though we're a little more permissive about
            //   children which are _lists_: we don't care about their max_len, we just take the max()
            let first_type = TypeSignature::type_of(first);
            let (mut max_len, dimension) = match first_type.list_dimensions {
                Some((max_len, dimension)) => (max_len, dimension + 1),
                None => (args.len() as u8, 1)
            };

            for x in rest {
                let x_type = TypeSignature::type_of(x);
                if let Some((child_max_len, child_dimension)) = x_type.list_dimensions {
                    // we're making a higher order list, so check the type more loosely.
                    if !(x_type.atomic_type == first_type.atomic_type &&
                         dimension == child_dimension + 1) {
                        return Err(Error::InvalidArguments(
                            format!("List must be composed of a single type. Expected {:?}. Found {:?}.",
                                    first_type, x_type)))
                    } else {
                        // otherwise, it matches, so make sure we expand max_len to fit the child list.
                        if child_max_len > max_len {
                            max_len = child_max_len;
                        }
                    }
                } else if x_type != first_type {
                    return Err(Error::InvalidArguments(
                        format!("List must be composed of a single type. Expected {:?}. Found {:?}.",
                                first_type, x_type)))
                }
            }

            Ok(TypeSignature { atomic_type: first_type.atomic_type,
                               list_dimensions: Some((max_len, dimension)) })
        } else {
            Ok(TypeSignature::get_empty_list_type())
        }
    }

    fn get_atom_type(typename: &str) -> Result<AtomTypeIdentifier> {
        match typename {
            "int" => Ok(AtomTypeIdentifier::IntType),
            "void" => Ok(AtomTypeIdentifier::VoidType),
            "bool" => Ok(AtomTypeIdentifier::BoolType),
            "buff" => Ok(AtomTypeIdentifier::BufferType),
            _ => Err(Error::ParseError(format!("Unknown type name: '{:?}'", typename)))
        }
    }

    
    fn get_list_type(prefix: &str, typename: &str, dimension: &str, max_len: &str) -> Result<TypeSignature> {
        if prefix != "list" {
            let message = format!("Unknown type name: '{}-{}-{}-{}'", prefix, typename, dimension, max_len);
            return Err(Error::ParseError(message))
        }
        let atom_type = TypeSignature::get_atom_type(typename)?;
        let dimension = match u8::from_str_radix(dimension, 10) {
            Ok(parsed) => Ok(parsed),
            Err(_e) => Err(Error::ParseError(
                format!("Failed to parse dimension of type: '{}-{}-{}-{}'",
                        prefix, typename, dimension, max_len)))
        }?;
        let max_len = match u8::from_str_radix(max_len, 10) {
            Ok(parsed) => Ok(parsed),
            Err(_e) => Err(Error::ParseError(
                format!("Failed to parse max_len of type: '{}-{}-{}-{}'",
                        prefix, typename, dimension, max_len)))
        }?;
        TypeSignature::new_list(atom_type, max_len, dimension)
    }

    // TODO: these type strings are limited to conveying lists of non-tuple types.
    pub fn parse_type_str(x: &str) -> Result<TypeSignature> {
        let components: Vec<_> = x.split('-').collect();
        match components.len() {
            1 => {
                let atom_type = TypeSignature::get_atom_type(components[0])?;
                Ok(TypeSignature::new_atom(atom_type))
            },
            4 => TypeSignature::get_list_type(components[0], components[1], components[2], components[3]),
            _ => Err(Error::ParseError(
                format!("Unknown type name: '{}'", x)))
        }
    }
}

impl DefinedFunction {
    pub fn new(body: SymbolicExpression, arguments: Vec<String>) -> DefinedFunction {
        DefinedFunction {
            body: body,
            arguments: arguments,
        }
    }

    pub fn apply(&self, args: &[Value], env: &mut Environment) -> InterpreterResult {
        let mut context = Context::new();

        let mut arg_iterator = self.arguments.iter().zip(args.iter());
        let _result = arg_iterator.try_for_each(|(arg, value)| {
            match context.variables.insert((*arg).clone(), (*value).clone()) {
                Some(_val) => Err(Error::InvalidArguments("Multiply defined function argument".to_string())),
                _ => Ok(())
            }
        })?;
        eval(&self.body, env, &context)
    }

    pub fn get_identifier(&self) -> FunctionIdentifier {
        return FunctionIdentifier {
            body: self.body.clone(),
            arguments: self.arguments.clone() }
    }
}
