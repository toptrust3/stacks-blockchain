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

define_named_enum!(ClarityCostFunction {
    AnalysisTypeAnnotate("cost_analysis_type_annotate"),
    AnalysisTypeCheck("cost_analysis_type_check"),
    AnalysisTypeLookup("cost_analysis_type_lookup"),
    AnalysisVisit("cost_analysis_visit"),
    AnalysisIterableFunc("cost_analysis_iterable_func"),
    AnalysisOptionCons("cost_analysis_option_cons"),
    AnalysisOptionCheck("cost_analysis_option_check"),
    AnalysisBindName("cost_analysis_bind_name"),
    AnalysisListItemsCheck("cost_analysis_list_items_check"),
    AnalysisCheckTupleGet("cost_analysis_check_tuple_get"),
    AnalysisCheckTupleMerge("cost_analysis_check_tuple_merge"),
    AnalysisCheckTupleCons("cost_analysis_check_tuple_cons"),
    AnalysisTupleItemsCheck("cost_analysis_tuple_items_check"),
    AnalysisCheckLet("cost_analysis_check_let"),
    AnalysisLookupFunction("cost_analysis_lookup_function"),
    AnalysisLookupFunctionTypes("cost_analysis_lookup_function_types"),
    AnalysisLookupVariableConst("cost_analysis_lookup_variable_const"),
    AnalysisLookupVariableDepth("cost_analysis_lookup_variable_depth"),
    AstParse("cost_ast_parse"),
    AstCycleDetection("cost_ast_cycle_detection"),
    AnalysisStorage("cost_analysis_storage"),
    AnalysisUseTraitEntry("cost_analysis_use_trait_entry"),
    AnalysisGetFunctionEntry("cost_analysis_get_function_entry"),
    AnalysisFetchContractEntry("cost_analysis_fetch_contract_entry"),
    LookupVariableDepth("cost_lookup_variable_depth"),
    LookupVariableSize("cost_lookup_variable_size"),
    LookupFunction("cost_lookup_function"),
    BindName("cost_bind_name"),
    InnerTypeCheckCost("cost_inner_type_check_cost"),
    UserFunctionApplication("cost_user_function_application"),
    Let("cost_let"),
    If("cost_if"),
    Asserts("cost_asserts"),
    Map("cost_map"),
    Filter("cost_filter"),
    Len("cost_len"),
    ElementAt("cost_element_at"),
    IndexOf("cost_index_of"),
    Fold("cost_fold"),
    ListCons("cost_list_cons"),
    TypeParseStep("cost_type_parse_step"),
    TupleGet("cost_tuple_get"),
    TupleMerge("cost_tuple_merge"),
    TupleCons("cost_tuple_cons"),
    Add("cost_add"),
    Sub("cost_sub"),
    Mul("cost_mul"),
    Div("cost_div"),
    Geq("cost_geq"),
    Leq("cost_leq"),
    Le("cost_le"),
    Ge("cost_ge"),
    IntCast("cost_int_cast"),
    Mod("cost_mod"),
    Pow("cost_pow"),
    Sqrti("cost_sqrti"),
    Log2("cost_log2"),
    Xor("cost_xor"),
    Not("cost_not"),
    Eq("cost_eq"),
    Begin("cost_begin"),
    Hash160("cost_hash160"),
    Sha256("cost_sha256"),
    Sha512("cost_sha512"),
    Sha512t256("cost_sha512t256"),
    Keccak256("cost_keccak256"),
    Secp256k1recover("cost_secp256k1recover"),
    Secp256k1verify("cost_secp256k1verify"),
    Print("cost_print"),
    SomeCons("cost_some_cons"),
    OkCons("cost_ok_cons"),
    ErrCons("cost_err_cons"),
    DefaultTo("cost_default_to"),
    UnwrapRet("cost_unwrap_ret"),
    UnwrapErrOrRet("cost_unwrap_err_or_ret"),
    IsOkay("cost_is_okay"),
    IsNone("cost_is_none"),
    IsErr("cost_is_err"),
    IsSome("cost_is_some"),
    Unwrap("cost_unwrap"),
    UnwrapErr("cost_unwrap_err"),
    TryRet("cost_try_ret"),
    Match("cost_match"),
    Or("cost_or"),
    And("cost_and"),
    Append("cost_append"),
    Concat("cost_concat"),
    AsMaxLen("cost_as_max_len"),
    ContractCall("cost_contract_call"),
    ContractOf("cost_contract_of"),
    PrincipalOf("cost_principal_of"),
    AtBlock("cost_at_block"),
    LoadContract("cost_load_contract"),
    CreateMap("cost_create_map"),
    CreateVar("cost_create_var"),
    CreateNft("cost_create_nft"),
    CreateFt("cost_create_ft"),
    FetchEntry("cost_fetch_entry"),
    SetEntry("cost_set_entry"),
    FetchVar("cost_fetch_var"),
    SetVar("cost_set_var"),
    ContractStorage("cost_contract_storage"),
    BlockInfo("cost_block_info"),
    StxBalance("cost_stx_balance"),
    StxTransfer("cost_stx_transfer"),
    FtMint("cost_ft_mint"),
    FtTransfer("cost_ft_transfer"),
    FtBalance("cost_ft_balance"),
    FtSupply("cost_ft_get_supply"),
    FtBurn("cost_ft_burn"),
    NftMint("cost_nft_mint"),
    NftTransfer("cost_nft_transfer"),
    NftOwner("cost_nft_owner"),
    PoisonMicroblock("poison_microblock"),
});
