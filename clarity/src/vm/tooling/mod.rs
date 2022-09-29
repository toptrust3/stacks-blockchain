use std::collections::{BTreeMap, HashMap, HashSet};

use super::{
    analysis::ContractAnalysis,
    contexts::GlobalContext,
    docs::contracts::{make_func_ref, ContractRef, ErrorCode, DOCS_GENERATION_EPOCH},
    types::TypeSignature,
    ClarityVersion, ContractContext, Value,
};
use crate::vm::{
    analysis::{run_analysis, CheckResult},
    costs::LimitedCostTracker,
    database::MemoryBackingStore,
    types::QualifiedContractIdentifier,
};
use stacks_common::{consts::CHAIN_ID_TESTNET, types::StacksEpochId};

/// Used by CLI tools like the docs generator. Not used in production
pub fn mem_type_check(
    snippet: &str,
    version: ClarityVersion,
    epoch: StacksEpochId,
) -> CheckResult<(Option<TypeSignature>, ContractAnalysis)> {
    use crate::vm::ast::parse;
    let contract_identifier = QualifiedContractIdentifier::transient();
    let mut contract = parse(&contract_identifier, snippet, version, epoch).unwrap();
    let mut marf = MemoryBackingStore::new();
    let mut analysis_db = marf.as_analysis_db();
    let cost_tracker = LimitedCostTracker::new_free();
    match run_analysis(
        &QualifiedContractIdentifier::transient(),
        &mut contract,
        &mut analysis_db,
        false,
        cost_tracker,
        version,
    ) {
        Ok(x) => {
            // return the first type result of the type checker
            let first_type = x
                .type_map
                .as_ref()
                .unwrap()
                .get_type(&x.expressions.last().unwrap())
                .cloned();
            Ok((first_type, x))
        }
        Err((e, _)) => Err(e),
    }
}

fn doc_execute(program: &str) -> Result<Option<Value>, crate::vm::Error> {
    let contract_id = QualifiedContractIdentifier::transient();
    let mut contract_context = ContractContext::new(contract_id.clone(), ClarityVersion::Clarity2);
    let mut marf = MemoryBackingStore::new();
    let conn = marf.as_clarity_db();
    let mut global_context = GlobalContext::new(
        false,
        CHAIN_ID_TESTNET,
        conn,
        LimitedCostTracker::new_free(),
        DOCS_GENERATION_EPOCH,
    );
    global_context.execute(|g| {
        let parsed = crate::vm::ast::build_ast(
            &contract_id,
            program,
            &mut (),
            ClarityVersion::latest(),
            StacksEpochId::latest(),
        )?
        .expressions;
        crate::vm::eval_all(&parsed, &mut contract_context, g, None)
    })
}

pub fn make_docs(content: &str, support_docs: &ContractSupportDocs) -> ContractRef {
    let (_, contract_analysis) =
        mem_type_check(content, ClarityVersion::latest(), StacksEpochId::latest())
            .expect("BUG: failed to type check boot contract");

    let ContractAnalysis {
        public_function_types,
        read_only_function_types,
        variable_types,
        ..
    } = contract_analysis;
    let public_functions: Vec<_> = public_function_types
        .iter()
        .filter(|(func_name, _)| !support_docs.skip_func_display.contains(func_name.as_str()))
        .map(|(func_name, func_type)| {
            let description = support_docs
                .descriptions
                .get(func_name.as_str())
                .expect(&format!("BUG: no description for {}", func_name.as_str()));
            make_func_ref(func_name, func_type, description)
        })
        .collect();

    let read_only_functions: Vec<_> = read_only_function_types
        .iter()
        .filter(|(func_name, _)| !support_docs.skip_func_display.contains(func_name.as_str()))
        .map(|(func_name, func_type)| {
            let description = support_docs
                .descriptions
                .get(func_name.as_str())
                .expect(&format!("BUG: no description for {}", func_name.as_str()));
            make_func_ref(func_name, func_type, description)
        })
        .collect();

    let ecode_names = variable_types
        .iter()
        .filter_map(|(var_name, _)| {
            if var_name.starts_with("ERR_") {
                Some(format!("{}: {}", var_name.as_str(), var_name.as_str()))
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let ecode_to_eval = format!("{}\n {{ {} }}", content, ecode_names);
    let ecode_result = doc_execute(&ecode_to_eval)
        .expect("BUG: failed to evaluate contract for constant value")
        .expect("BUG: failed to return constant value")
        .expect_tuple();

    let error_codes = variable_types
        .iter()
        .filter_map(|(var_name, type_signature)| {
            if var_name.starts_with("ERR_") {
                let value = ecode_result
                    .get(var_name)
                    .expect("BUG: failed to fetch tuple entry from ecode output")
                    .to_string();
                Some(ErrorCode {
                    name: var_name.to_string(),
                    value,
                    value_type: type_signature.to_string(),
                })
            } else {
                None
            }
        })
        .collect();

    ContractRef {
        public_functions,
        read_only_functions,
        error_codes,
    }
}

/// Produce a set of documents for multiple contracts, supplied as a list of `(contract_name, contract_content)` pairs,
///  and a map from `contract_name` to corresponding `ContractSupportDocs`
pub fn produce_docs_refs<A: AsRef<str>, B: AsRef<str>>(
    contracts: &[(A, B)],
    support_docs: &HashMap<&str, ContractSupportDocs>,
) -> BTreeMap<String, ContractRef> {
    let mut docs = BTreeMap::new();

    for (contract_name, content) in contracts.iter() {
        if let Some(contract_support) = support_docs.get(contract_name.as_ref()) {
            let contract_ref = make_docs(content.as_ref(), contract_support);

            docs.insert(contract_name.as_ref().to_string(), contract_ref);
        }
    }

    docs
}

pub struct ContractSupportDocs {
    pub descriptions: HashMap<&'static str, &'static str>,
    pub skip_func_display: HashSet<&'static str>,
}
