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

pub mod constants;
pub mod cost_functions;

use regex::internal::Exec;
use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use std::convert::{TryFrom, TryInto};
use std::{cmp, fmt};

use std::collections::{BTreeMap, HashMap};

use chainstate::stacks::boot::STACKS_BOOT_COST_CONTRACT;

use vm::ast::ContractAST;
use vm::contexts::{ContractContext, Environment, GlobalContext, OwnedEnvironment};
use vm::costs::cost_functions::ClarityCostFunction;
use vm::database::{marf::NullBackingStore, ClarityDatabase, MemoryBackingStore};
use vm::errors::{Error, InterpreterResult};
use vm::types::Value::UInt;
use vm::types::{QualifiedContractIdentifier, TypeSignature, NONE};
use vm::{ast, eval_all, SymbolicExpression, Value};

type Result<T> = std::result::Result<T, CostErrors>;

pub const CLARITY_MEMORY_LIMIT: u64 = 100 * 1000 * 1000;

pub fn runtime_cost<T: TryInto<u64>, C: CostTracker>(
    cost_function: ClarityCostFunction,
    tracker: &mut C,
    input: T,
) -> Result<()> {
    let size: u64 = input.try_into().map_err(|_| CostErrors::CostOverflow)?;
    let cost = tracker.compute_cost(cost_function, size)?;

    tracker.add_cost(cost)
}

macro_rules! finally_drop_memory {
    ( $env: expr, $used_mem:expr; $exec:expr ) => {{
        let result = (|| $exec)();
        $env.drop_memory($used_mem);
        result
    }};
}

pub fn analysis_typecheck_cost<T: CostTracker>(
    track: &mut T,
    t1: &TypeSignature,
    t2: &TypeSignature,
) -> Result<()> {
    let t1_size = t1.type_size().map_err(|_| CostErrors::CostOverflow)?;
    let t2_size = t2.type_size().map_err(|_| CostErrors::CostOverflow)?;
    let cost = track.compute_cost(
        ClarityCostFunction::AnalysisTypeCheck,
        cmp::max(t1_size, t2_size) as u64,
    )?;
    track.add_cost(cost)
}

pub trait MemoryConsumer {
    fn get_memory_use(&self) -> u64;
}

impl MemoryConsumer for Value {
    fn get_memory_use(&self) -> u64 {
        self.size().into()
    }
}

pub trait CostTracker {
    fn compute_cost(
        &mut self,
        cost_function: ClarityCostFunction,
        input: u64,
    ) -> Result<ExecutionCost>;
    fn add_cost(&mut self, cost: ExecutionCost) -> Result<()>;
    fn add_memory(&mut self, memory: u64) -> Result<()>;
    fn drop_memory(&mut self, memory: u64);
    fn reset_memory(&mut self);
}

// Don't track!
impl CostTracker for () {
    fn compute_cost(
        &mut self,
        _cost_function: ClarityCostFunction,
        _input: u64,
    ) -> std::result::Result<ExecutionCost, CostErrors> {
        Ok(ExecutionCost::zero())
    }
    fn add_cost(&mut self, _cost: ExecutionCost) -> std::result::Result<(), CostErrors> {
        Ok(())
    }
    fn add_memory(&mut self, _memory: u64) -> std::result::Result<(), CostErrors> {
        Ok(())
    }
    fn drop_memory(&mut self, _memory: u64) {}
    fn reset_memory(&mut self) {}
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct ClarityCostFunctionReference {
    pub contract_id: QualifiedContractIdentifier,
    pub function_name: String,
}

impl ::std::fmt::Display for ClarityCostFunctionReference {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        write!(f, "{}.{}", &self.contract_id, &self.function_name)
    }
}

impl ClarityCostFunctionReference {
    fn new(id: QualifiedContractIdentifier, name: String) -> ClarityCostFunctionReference {
        ClarityCostFunctionReference {
            contract_id: id,
            function_name: name,
        }
    }
}

type ClarityCostContract = &'static str;

#[derive(Clone)]
pub struct LimitedCostTracker {
    cost_function_references: HashMap<&'static ClarityCostFunction, ClarityCostFunctionReference>,
    cost_contracts: HashMap<QualifiedContractIdentifier, ContractContext>,
    total: ExecutionCost,
    limit: ExecutionCost,
    memory: u64,
    memory_limit: u64,
    free: bool,
}

impl fmt::Debug for LimitedCostTracker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LimitedCostTracker")
            .field("total", &self.total)
            .field("limit", &self.limit)
            .field("memory", &self.memory)
            .field("memory_limit", &self.memory_limit)
            .field("free", &self.free)
            .finish()
    }
}
impl PartialEq for LimitedCostTracker {
    fn eq(&self, other: &Self) -> bool {
        self.total == other.total
            && self.limit == other.limit
            && self.memory == other.memory
            && self.memory_limit == other.memory_limit
            && self.free == other.free
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CostErrors {
    CostComputationFailed(String),
    CostOverflow,
    CostBalanceExceeded(ExecutionCost, ExecutionCost),
    MemoryBalanceExceeded(u64, u64),
    CostContractLoadFailure,
}

impl LimitedCostTracker {
    pub fn new(
        limit: ExecutionCost,
        clarity_db: &mut ClarityDatabase,
    ) -> Result<LimitedCostTracker> {
        let mut cost_tracker = LimitedCostTracker {
            cost_function_references: HashMap::new(),
            cost_contracts: HashMap::new(),
            limit,
            memory_limit: CLARITY_MEMORY_LIMIT,
            total: ExecutionCost::zero(),
            memory: 0,
            free: false,
        };
        cost_tracker.load_boot_costs(clarity_db)?;
        Ok(cost_tracker)
    }
    pub fn new_max_limit(clarity_db: &mut ClarityDatabase) -> Result<LimitedCostTracker> {
        let mut cost_tracker = LimitedCostTracker {
            cost_function_references: HashMap::new(),
            cost_contracts: HashMap::new(),
            limit: ExecutionCost::max_value(),
            total: ExecutionCost::zero(),
            memory: 0,
            memory_limit: CLARITY_MEMORY_LIMIT,
            free: false,
        };
        cost_tracker.load_boot_costs(clarity_db)?;
        Ok(cost_tracker)
    }
    pub fn new_free() -> LimitedCostTracker {
        LimitedCostTracker {
            cost_function_references: HashMap::new(),
            cost_contracts: HashMap::new(),
            limit: ExecutionCost::max_value(),
            total: ExecutionCost::zero(),
            memory: 0,
            memory_limit: CLARITY_MEMORY_LIMIT,
            free: true,
        }
    }
    pub fn load_boot_costs(&mut self, clarity_db: &mut ClarityDatabase) -> Result<()> {
        let boot_costs_id = (*STACKS_BOOT_COST_CONTRACT).clone();

        clarity_db.begin();

        let mut cost_contracts = HashMap::new();
        let mut m = HashMap::new();
        for f in ClarityCostFunction::ALL.iter() {
            m.insert(
                f,
                ClarityCostFunctionReference::new(boot_costs_id.clone(), f.get_name()),
            );
            if !cost_contracts.contains_key(&boot_costs_id) {
                let contract_context = match clarity_db.get_contract(&boot_costs_id) {
                    Ok(contract) => contract.contract_context,
                    Err(e) => {
                        error!("Failed to load intended Clarity cost contract";
                               "contract" => %boot_costs_id.to_string(),
                               "error" => %format!("{:?}", e));
                        clarity_db.roll_back();
                        return Err(CostErrors::CostContractLoadFailure);
                    }
                };
                cost_contracts.insert(boot_costs_id.clone(), contract_context);
            }
        }
        self.cost_function_references = m;
        self.cost_contracts = cost_contracts;

        clarity_db.roll_back();

        return Ok(());
    }
    pub fn get_total(&self) -> ExecutionCost {
        self.total.clone()
    }
    pub fn set_total(&mut self, total: ExecutionCost) -> () {
        // used by the miner to "undo" the cost of a transaction when trying to pack a block.
        self.total = total;
    }
    pub fn get_limit(&self) -> ExecutionCost {
        self.limit.clone()
    }
}

fn parse_cost(
    cost_function: ClarityCostFunction,
    eval_result: InterpreterResult<Option<Value>>,
) -> Result<ExecutionCost> {
    match eval_result {
        Ok(Some(Value::Tuple(data))) => {
            let results = (
                data.data_map.get("write_length"),
                data.data_map.get("write_count"),
                data.data_map.get("runtime"),
                data.data_map.get("read_length"),
                data.data_map.get("read_count"),
            );

            match results {
                (
                    Some(UInt(write_length)),
                    Some(UInt(write_count)),
                    Some(UInt(runtime)),
                    Some(UInt(read_length)),
                    Some(UInt(read_count)),
                ) => Ok(ExecutionCost {
                    write_length: (*write_length as u64),
                    write_count: (*write_count as u64),
                    runtime: (*runtime as u64),
                    read_length: (*read_length as u64),
                    read_count: (*read_count as u64),
                }),
                _ => Err(CostErrors::CostComputationFailed(
                    "Execution Cost tuple does not contain only UInts".to_string(),
                )),
            }
        }
        Ok(Some(_)) => Err(CostErrors::CostComputationFailed(
            "Clarity cost function returned something other than a Cost tuple".to_string(),
        )),
        Ok(None) => Err(CostErrors::CostComputationFailed(
            "Clarity cost function returned nothing".to_string(),
        )),
        Err(e) => Err(CostErrors::CostComputationFailed(format!(
            "Error evaluating result of cost function {}: {}",
            cost_function.get_name(),
            e
        ))),
    }
}

fn compute_cost(
    cost_tracker: &mut LimitedCostTracker,
    cost_function: ClarityCostFunction,
    input_size: u64,
) -> Result<ExecutionCost> {
    let mut null_store = NullBackingStore::new();
    let conn = null_store.as_clarity_db();
    let mut global_context = GlobalContext::new(conn, LimitedCostTracker::new_free());

    let cost_function_reference = cost_tracker
        .cost_function_references
        .get(&cost_function)
        .ok_or(CostErrors::CostComputationFailed(format!(
            "CostFunction not defined: {}",
            &cost_function
        )))?
        .clone();

    let cost_contract = cost_tracker
        .cost_contracts
        .get_mut(&cost_function_reference.contract_id)
        .ok_or(CostErrors::CostComputationFailed(format!(
            "CostFunction not found: {} at {}",
            &cost_function, &cost_function_reference
        )))?;

    let program = vec![
        SymbolicExpression::atom(cost_function_reference.function_name[..].into()),
        SymbolicExpression::atom_value(Value::UInt(input_size.into())),
    ];

    let function_invocation = [SymbolicExpression::list(program.into_boxed_slice())];

    let eval_result = eval_all(&function_invocation, cost_contract, &mut global_context);

    parse_cost(cost_function, eval_result)
}

fn add_cost(
    s: &mut LimitedCostTracker,
    cost: ExecutionCost,
) -> std::result::Result<(), CostErrors> {
    s.total.add(&cost)?;
    if s.total.exceeds(&s.limit) {
        Err(CostErrors::CostBalanceExceeded(
            s.total.clone(),
            s.limit.clone(),
        ))
    } else {
        Ok(())
    }
}

fn add_memory(s: &mut LimitedCostTracker, memory: u64) -> std::result::Result<(), CostErrors> {
    s.memory = s.memory.cost_overflow_add(memory)?;
    if s.memory > s.memory_limit {
        Err(CostErrors::MemoryBalanceExceeded(s.memory, s.memory_limit))
    } else {
        Ok(())
    }
}

fn drop_memory(s: &mut LimitedCostTracker, memory: u64) {
    s.memory = s
        .memory
        .checked_sub(memory)
        .expect("Underflowed dropped memory");
}

impl CostTracker for LimitedCostTracker {
    fn compute_cost(
        &mut self,
        cost_function: ClarityCostFunction,
        input: u64,
    ) -> std::result::Result<ExecutionCost, CostErrors> {
        if self.free {
            return Ok(ExecutionCost::zero());
        }
        compute_cost(self, cost_function, input)
    }
    fn add_cost(&mut self, cost: ExecutionCost) -> std::result::Result<(), CostErrors> {
        if self.free {
            return Ok(());
        }
        add_cost(self, cost)
    }
    fn add_memory(&mut self, memory: u64) -> std::result::Result<(), CostErrors> {
        if self.free {
            return Ok(());
        }
        add_memory(self, memory)
    }
    fn drop_memory(&mut self, memory: u64) {
        if !self.free {
            drop_memory(self, memory)
        }
    }
    fn reset_memory(&mut self) {
        if !self.free {
            self.memory = 0;
        }
    }
}

impl CostTracker for &mut LimitedCostTracker {
    fn compute_cost(
        &mut self,
        cost_function: ClarityCostFunction,
        input: u64,
    ) -> std::result::Result<ExecutionCost, CostErrors> {
        if self.free {
            return Ok(ExecutionCost::zero());
        }
        compute_cost(self, cost_function, input)
    }
    fn add_cost(&mut self, cost: ExecutionCost) -> std::result::Result<(), CostErrors> {
        if self.free {
            return Ok(());
        }
        add_cost(self, cost)
    }
    fn add_memory(&mut self, memory: u64) -> std::result::Result<(), CostErrors> {
        if self.free {
            return Ok(());
        }
        add_memory(self, memory)
    }
    fn drop_memory(&mut self, memory: u64) {
        if !self.free {
            drop_memory(self, memory)
        }
    }
    fn reset_memory(&mut self) {
        if !self.free {
            self.memory = 0;
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub enum CostFunctions {
    Constant(u64),
    Linear(u64, u64),
    NLogN(u64, u64),
    LogN(u64, u64),
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct SimpleCostSpecification {
    pub write_count: CostFunctions,
    pub write_length: CostFunctions,
    pub read_count: CostFunctions,
    pub read_length: CostFunctions,
    pub runtime: CostFunctions,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct ExecutionCost {
    pub write_length: u64,
    pub write_count: u64,
    pub read_length: u64,
    pub read_count: u64,
    pub runtime: u64,
}

impl fmt::Display for ExecutionCost {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{{\"runtime\": {}, \"write_length\": {}, \"write_count\": {}, \"read_length\": {}, \"read_count\": {}}}",
               self.runtime, self.write_length, self.write_count, self.read_length, self.read_count)
    }
}

impl ToSql for ExecutionCost {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput> {
        let val = serde_json::to_string(self).expect("FAIL: could not serialize ExecutionCost");
        Ok(ToSqlOutput::from(val))
    }
}

impl FromSql for ExecutionCost {
    fn column_result(value: ValueRef) -> FromSqlResult<ExecutionCost> {
        let str_val = String::column_result(value)?;
        let parsed = serde_json::from_str(&str_val)
            .expect("CORRUPTION: failed to parse ExecutionCost from DB");
        Ok(parsed)
    }
}

pub trait CostOverflowingMath<T> {
    fn cost_overflow_mul(self, other: T) -> Result<T>;
    fn cost_overflow_add(self, other: T) -> Result<T>;
    fn cost_overflow_sub(self, other: T) -> Result<T>;
}

impl CostOverflowingMath<u64> for u64 {
    fn cost_overflow_mul(self, other: u64) -> Result<u64> {
        self.checked_mul(other)
            .ok_or_else(|| CostErrors::CostOverflow)
    }
    fn cost_overflow_add(self, other: u64) -> Result<u64> {
        self.checked_add(other)
            .ok_or_else(|| CostErrors::CostOverflow)
    }
    fn cost_overflow_sub(self, other: u64) -> Result<u64> {
        self.checked_sub(other)
            .ok_or_else(|| CostErrors::CostOverflow)
    }
}

impl ExecutionCost {
    pub fn zero() -> ExecutionCost {
        Self {
            runtime: 0,
            write_length: 0,
            read_count: 0,
            write_count: 0,
            read_length: 0,
        }
    }

    pub fn max_value() -> ExecutionCost {
        Self {
            runtime: u64::max_value(),
            write_length: u64::max_value(),
            read_count: u64::max_value(),
            write_count: u64::max_value(),
            read_length: u64::max_value(),
        }
    }

    pub fn runtime(runtime: u64) -> ExecutionCost {
        Self {
            runtime,
            write_length: 0,
            read_count: 0,
            write_count: 0,
            read_length: 0,
        }
    }

    pub fn add_runtime(&mut self, runtime: u64) -> Result<()> {
        self.runtime = self.runtime.cost_overflow_add(runtime)?;
        Ok(())
    }

    pub fn add(&mut self, other: &ExecutionCost) -> Result<()> {
        self.runtime = self.runtime.cost_overflow_add(other.runtime)?;
        self.read_count = self.read_count.cost_overflow_add(other.read_count)?;
        self.read_length = self.read_length.cost_overflow_add(other.read_length)?;
        self.write_length = self.write_length.cost_overflow_add(other.write_length)?;
        self.write_count = self.write_count.cost_overflow_add(other.write_count)?;
        Ok(())
    }

    pub fn sub(&mut self, other: &ExecutionCost) -> Result<()> {
        self.runtime = self.runtime.cost_overflow_sub(other.runtime)?;
        self.read_count = self.read_count.cost_overflow_sub(other.read_count)?;
        self.read_length = self.read_length.cost_overflow_sub(other.read_length)?;
        self.write_length = self.write_length.cost_overflow_sub(other.write_length)?;
        self.write_count = self.write_count.cost_overflow_sub(other.write_count)?;
        Ok(())
    }

    pub fn multiply(&mut self, times: u64) -> Result<()> {
        self.runtime = self.runtime.cost_overflow_mul(times)?;
        self.read_count = self.read_count.cost_overflow_mul(times)?;
        self.read_length = self.read_length.cost_overflow_mul(times)?;
        self.write_length = self.write_length.cost_overflow_mul(times)?;
        self.write_count = self.write_count.cost_overflow_mul(times)?;
        Ok(())
    }

    /// Returns whether or not this cost exceeds any dimension of the
    ///  other cost.
    pub fn exceeds(&self, other: &ExecutionCost) -> bool {
        self.runtime > other.runtime
            || self.write_length > other.write_length
            || self.write_count > other.write_count
            || self.read_count > other.read_count
            || self.read_length > other.read_length
    }

    pub fn max_cost(first: ExecutionCost, second: ExecutionCost) -> ExecutionCost {
        Self {
            runtime: first.runtime.max(second.runtime),
            write_length: first.write_length.max(second.write_length),
            write_count: first.write_count.max(second.write_count),
            read_count: first.read_count.max(second.read_count),
            read_length: first.read_length.max(second.read_length),
        }
    }
}

// ONLY WORKS IF INPUT IS u64
fn int_log2(input: u64) -> Option<u64> {
    63_u32.checked_sub(input.leading_zeros()).map(|floor_log| {
        if input.trailing_zeros() == floor_log {
            u64::from(floor_log)
        } else {
            u64::from(floor_log + 1)
        }
    })
}

impl CostFunctions {
    pub fn compute_cost(&self, input: u64) -> Result<u64> {
        match self {
            CostFunctions::Constant(val) => Ok(*val),
            CostFunctions::Linear(a, b) => a.cost_overflow_mul(input)?.cost_overflow_add(*b),
            CostFunctions::LogN(a, b) => {
                // a*log(input)) + b
                //  and don't do log(0).
                int_log2(cmp::max(input, 1))
                    .ok_or_else(|| CostErrors::CostOverflow)?
                    .cost_overflow_mul(*a)?
                    .cost_overflow_add(*b)
            }
            CostFunctions::NLogN(a, b) => {
                // a*input*log(input)) + b
                //  and don't do log(0).
                int_log2(cmp::max(input, 1))
                    .ok_or_else(|| CostErrors::CostOverflow)?
                    .cost_overflow_mul(input)?
                    .cost_overflow_mul(*a)?
                    .cost_overflow_add(*b)
            }
        }
    }
}

impl SimpleCostSpecification {
    pub fn compute_cost(&self, input: u64) -> Result<ExecutionCost> {
        Ok(ExecutionCost {
            write_length: self.write_length.compute_cost(input)?,
            write_count: self.write_count.compute_cost(input)?,
            read_count: self.read_count.compute_cost(input)?,
            read_length: self.read_length.compute_cost(input)?,
            runtime: self.runtime.compute_cost(input)?,
        })
    }
}

impl From<ExecutionCost> for SimpleCostSpecification {
    fn from(value: ExecutionCost) -> SimpleCostSpecification {
        let ExecutionCost {
            write_length,
            write_count,
            read_count,
            read_length,
            runtime,
        } = value;
        SimpleCostSpecification {
            write_length: CostFunctions::Constant(write_length),
            write_count: CostFunctions::Constant(write_count),
            read_length: CostFunctions::Constant(read_length),
            read_count: CostFunctions::Constant(read_count),
            runtime: CostFunctions::Constant(runtime),
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn test_simple_overflows() {
        assert_eq!(
            u64::max_value().cost_overflow_add(1),
            Err(CostErrors::CostOverflow)
        );
        assert_eq!(
            u64::max_value().cost_overflow_mul(2),
            Err(CostErrors::CostOverflow)
        );
        assert_eq!(
            CostFunctions::NLogN(1, 1).compute_cost(u64::max_value()),
            Err(CostErrors::CostOverflow)
        );
    }

    #[test]
    fn test_simple_sub() {
        assert_eq!(0u64.cost_overflow_sub(1), Err(CostErrors::CostOverflow));
    }

    #[test]
    fn test_simple_log2s() {
        let inputs = [
            1,
            2,
            4,
            8,
            16,
            31,
            32,
            33,
            39,
            64,
            128,
            2_u64.pow(63),
            u64::max_value(),
        ];
        let expected = [0, 1, 2, 3, 4, 5, 5, 6, 6, 6, 7, 63, 64];
        for (input, expected) in inputs.iter().zip(expected.iter()) {
            assert_eq!(int_log2(*input).unwrap(), *expected);
        }
    }
}
