use std::collections::HashMap;

use errors::Error;
use InterpreterResult;
use types::{ValueType, TypeSignature, TupleTypeSignature, AtomTypeIdentifier};

pub trait DataMap {
    fn fetch_entry(&self, key: &ValueType) -> InterpreterResult;
    fn set_entry(&mut self, key: ValueType, value: ValueType) -> Result<(), Error>;
    fn insert_entry(&mut self, key: ValueType, value: ValueType) -> InterpreterResult;
    fn delete_entry(&mut self, key: &ValueType) -> InterpreterResult;
}

pub trait ContractDatabase {
    fn get_data_map(&mut self, map_name: &str) -> Option<&mut DataMap>;
    fn create_map(&mut self, map_name: &str, key_type: TupleTypeSignature, value_type: TupleTypeSignature);
}

pub struct MemoryDataMap {
    map: HashMap<ValueType, ValueType>,
    key_type: TypeSignature,
    value_type: TypeSignature
}

pub struct MemoryContractDatabase {
    maps: HashMap<String, MemoryDataMap>,
}

impl MemoryDataMap {
    pub fn new(key_type: TupleTypeSignature,
               value_type: TupleTypeSignature) -> MemoryDataMap {
        MemoryDataMap {
            map: HashMap::new(),
            key_type: TypeSignature::new(
                AtomTypeIdentifier::TupleType(key_type), 0),
            value_type: TypeSignature::new(
                AtomTypeIdentifier::TupleType(value_type), 0)
        }
    }
}

impl MemoryContractDatabase {
    pub fn new() -> MemoryContractDatabase {
        MemoryContractDatabase { maps: HashMap::new() }
    }
}

impl ContractDatabase for MemoryContractDatabase {
    fn get_data_map(&mut self, map_name: &str) -> Option<&mut DataMap> {
        if let Some(data_map) = self.maps.get_mut(map_name) {
            Some(data_map)
        } else {
            None
        }
    }

    fn create_map(&mut self, map_name: &str, key_type: TupleTypeSignature, value_type: TupleTypeSignature) {
        let new_map = MemoryDataMap::new(key_type, value_type);
        self.maps.insert(map_name.to_string(), new_map);
    }
}

impl DataMap for MemoryDataMap {
    // TODO: currently, the return types and behavior of these functions are defined here,
    //   however, they should really be specified in the functions/database.rs file, whereas
    //   this file should really just be speccing out the database connection/requirement.

    fn fetch_entry(&self, key: &ValueType) -> InterpreterResult {
        let key_type = TypeSignature::type_of(key);
        if self.key_type != key_type {
            return Err(Error::TypeError(format!("{:?}", self.key_type), (*key).clone()))
        }
        if let Some(value) = self.map.get(key) {
            return Ok((*value).clone())
        } else {
            return Ok(ValueType::VoidType)
        }
    }

    fn set_entry(&mut self, key: ValueType, value: ValueType) -> Result<(), Error> {
        let key_type = TypeSignature::type_of(&key);
        if self.key_type != key_type {
            return Err(Error::TypeError(format!("{:?}", self.key_type), key))
        }
        let value_type = TypeSignature::type_of(&value);
        if self.value_type != value_type {
            return Err(Error::TypeError(format!("{:?}", self.value_type), value))
        }
        self.map.insert(key, value);
        Ok(())
    }

    fn insert_entry(&mut self, key: ValueType, value: ValueType) -> InterpreterResult {
        let key_type = TypeSignature::type_of(&key);
        if self.key_type != key_type {
            return Err(Error::TypeError(format!("{:?}", self.key_type), key))
        }
        let value_type = TypeSignature::type_of(&value);
        if self.value_type != value_type {
            return Err(Error::TypeError(format!("{:?}", self.value_type), value))
        }
        if self.map.contains_key(&key) {
            Ok(ValueType::BoolType(false))
        } else {
            self.map.insert(key, value);
            Ok(ValueType::BoolType(true))
        }
    }

    fn delete_entry(&mut self, key: &ValueType) -> InterpreterResult {
        let key_type = TypeSignature::type_of(key);
        if self.key_type != key_type {
            return Err(Error::TypeError(format!("{:?}", self.key_type), (*key).clone()))
        }
        if let Some(_value) = self.map.remove(key) {
            Ok(ValueType::BoolType(true))
        } else {
            Ok(ValueType::BoolType(false))
        }
    }
}

