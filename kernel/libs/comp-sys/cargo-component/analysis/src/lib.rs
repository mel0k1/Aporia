#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_lint;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

mod conf;

use std::collections::HashSet;

pub use conf::{init as init_conf, lookup_conf_file};
use rustc_middle::{
    mir::{
        ConstOperand, InlineAsmOperand, LocalDecl, Operand, Rvalue, Statement, StatementKind,
        Terminator, TerminatorKind,
    },
    ty::{InstanceKind, TyCtxt, TyKind},
};
use rustc_span::{
    Span,
    def_id::{DefId, LOCAL_CRATE, LocalDefId},
    symbol::Symbol,
};

const TOOL_NAME: &'static str = "component_access_control";
const CONTROLLED_ATTR: &'static str = "controlled";

pub fn enter_analysis<'tcx>(tcx: TyCtxt<'tcx>) {
    for mir_key in tcx.mir_keys(()) {
        check_body_mir(mir_key.clone(), tcx)
    }
}

fn check_body_mir(mir_key: LocalDefId, tcx: TyCtxt<'_>) {
    let body = tcx.instance_mir(InstanceKind::Item(mir_key.to_def_id()));

    let mut checked_def_ids = HashSet::new();
    for basic_block_data in body.basic_blocks.iter() {
        for statement in &basic_block_data.statements {
            check_statement(statement, tcx, &mut checked_def_ids);
        }

        if let Some(terminator) = &basic_block_data.terminator {
            check_terminator(terminator, tcx, &mut checked_def_ids);
        }
    }

    for local_decl in body.local_decls.iter() {
        check_local_decl(local_decl, tcx, &checked_def_ids)
    }
}

fn check_statement(statement: &Statement, tcx: TyCtxt<'_>, checked_def_ids: &mut HashSet<DefId>) {
    let mut def_paths = Vec::new();
    if let StatementKind::Assign(assignment) = &statement.kind {
        let rvalue = &assignment.1;
        match rvalue {
            Rvalue::Use(operand, _)
            | Rvalue::Repeat(operand, _)
            | Rvalue::Cast(_, operand, _)
            | Rvalue::UnaryOp(_, operand) => {
                check_invalid_operand(operand, tcx, &mut def_paths, checked_def_ids);
            }
            Rvalue::BinaryOp(_, two_operands) => {
                check_invalid_operand(&two_operands.0, tcx, &mut def_paths, checked_def_ids);
                check_invalid_operand(&two_operands.1, tcx, &mut def_paths, checked_def_ids);
            }
            Rvalue::Aggregate(_, operands) => {
                for operand in operands {
                    check_invalid_operand(operand, tcx, &mut def_paths, checked_def_ids);
                }
            }
            _ => {}
        }
    }
    let crate_symbol = tcx.crate_name(LOCAL_CRATE);
    let crate_name = crate_symbol.as_str();
    emit_note(tcx, statement.source_info.span, crate_name, def_paths)
}

fn check_terminator(
    terminator: &Terminator,
    tcx: TyCtxt<'_>,
    checked_def_ids: &mut HashSet<DefId>,
) {
    let mut def_paths = Vec::new();
    match &terminator.kind {
        TerminatorKind::SwitchInt { discr: operand, .. }
        | TerminatorKind::Assert { cond: operand, .. }
        | TerminatorKind::Yield { value: operand, .. } => {
            check_invalid_operand(operand, tcx, &mut def_paths, checked_def_ids);
        }
        TerminatorKind::Call { func, args, .. } => {
            check_invalid_operand(func, tcx, &mut def_paths, checked_def_ids);
            for arg in args {
                check_invalid_operand(&arg.node, tcx, &mut def_paths, checked_def_ids);
            }
        }
        TerminatorKind::InlineAsm { operands, .. } => {
            for asm_operand in operands {
                check_inline_asm_operand(&asm_operand, tcx, &mut def_paths, checked_def_ids);
            }
        }
        _ => {}
    }
    let crate_symbol = tcx.crate_name(LOCAL_CRATE);
    let crate_name = crate_symbol.as_str();
    emit_note(tcx, terminator.source_info.span, crate_name, def_paths)
}

fn check_local_decl(local_decl: &LocalDecl<'_>, tcx: TyCtxt<'_>, checked_def_ids: &HashSet<DefId>) {
    let ty = local_decl.ty;
    let def_id = if let TyKind::FnDef(def_id, ..) = ty.kind() {
        *def_id
    } else {
        return;
    };
    if checked_def_ids.contains(&def_id) {
        return;
    }
    let crate_symbol = tcx.crate_name(LOCAL_CRATE);
    let crate_name = crate_symbol.as_str();
    if let Some(def_path) = def_path_if_invalid_access(def_id, tcx) {
        emit_note(tcx, local_decl.source_info.span, crate_name, vec![def_path]);
    }
}

fn check_inline_asm_operand(
    asm_operand: &InlineAsmOperand<'_>,
    tcx: TyCtxt<'_>,
    def_paths: &mut Vec<String>,
    checked_def_ids: &mut HashSet<DefId>,
) {
    match asm_operand {
        InlineAsmOperand::In { value: operand, .. }
        | InlineAsmOperand::InOut {
            in_value: operand, ..
        } => {
            check_invalid_operand(operand, tcx, def_paths, checked_def_ids);
        }
        InlineAsmOperand::Const { value } | InlineAsmOperand::SymFn { value } => {
            check_constant(value, tcx, def_paths, checked_def_ids);
        }
        _ => {}
    }
}

fn check_invalid_operand(
    operand: &Operand,
    tcx: TyCtxt<'_>,
    def_paths: &mut Vec<String>,
    checked_def_ids: &mut HashSet<DefId>,
) {
    if let Operand::Constant(constant) = operand {
        check_constant(&constant, tcx, def_paths, checked_def_ids);
    } else {
        return;
    };
}

fn check_constant(
    constant: &ConstOperand<'_>,
    tcx: TyCtxt<'_>,
    def_paths: &mut Vec<String>,
    checked_def_ids: &mut HashSet<DefId>,
) {
    let def_id = if let Some(def_id) = constant.check_static_ptr(tcx) {
        def_id
    } else {
        let ty = constant.const_.ty();
        if let TyKind::FnDef(def_id, ..) = ty.kind() {
            *def_id
        } else {
            return;
        }
    };
    checked_def_ids.insert(def_id);

    if let Some(def_path) = def_path_if_invalid_access(def_id, tcx) {
        def_paths.push(def_path);
    }
}

fn def_path_if_invalid_access(def_id: DefId, tcx: TyCtxt<'_>) -> Option<String> {
    if def_id.is_local() {
        return None;
    }
    if !contains_controlled_attr(def_id, tcx) {
        return None;
    }
    def_path_if_not_in_whitelist(def_id, tcx)
}

fn def_path_if_not_in_whitelist(def_id: DefId, tcx: TyCtxt<'_>) -> Option<String> {
    let crate_symbol = tcx.crate_name(LOCAL_CRATE);
    let crate_name = crate_symbol.as_str();
    let def_path_str = def_path_for_def_id(tcx, def_id);
    if conf::CONFIG
        .get()
        .unwrap()
        .allow_access(crate_name, &def_path_str)
    {
        None
    } else {
        Some(def_path_str)
    }
}

fn contains_controlled_attr(def_id: DefId, tcx: TyCtxt<'_>) -> bool {
    tcx.tool_attrs(
        def_id,
        &[Symbol::intern(TOOL_NAME), Symbol::intern(CONTROLLED_ATTR)],
    )
    .next()
    .is_some()
}

fn def_path_for_def_id(tcx: TyCtxt<'_>, def_id: DefId) -> String {
    match tcx.impl_of_assoc(def_id) {
        None => common_def_path_str(tcx, def_id),
        Some(impl_def_id) => def_path_str_for_impl(tcx, def_id, impl_def_id),
    }
}

fn common_def_path_str(tcx: TyCtxt<'_>, def_id: DefId) -> String {
    let def_path = tcx.def_path(def_id);
    let crate_name = tcx.crate_name(def_path.krate);
    format!("{}{}", crate_name, def_path.to_string_no_crate_verbose())
}

fn def_path_str_for_impl(tcx: TyCtxt<'_>, def_id: DefId, impl_def_id: DefId) -> String {
    let item_name = tcx.item_name(def_id).to_string();
    let impl_ty = tcx.type_of(impl_def_id).skip_binder();
    let impl_ty_def_id = impl_ty
        .ty_adt_def()
        .expect("Method should impl an adt type")
        .did();
    let impl_ty_name = common_def_path_str(tcx, impl_ty_def_id);
    format!("{impl_ty_name}::{item_name}")
}

fn emit_note(tcx: TyCtxt<'_>, span: Span, crate_name: &str, def_paths: Vec<String>) {
    if def_paths.len() > 0 {
        const TITLE: &'static str = "access controlled entry point is disallowed";
        let def_path = def_paths.join(", ");
        let warning_message = format!("access {} in {}", def_path, crate_name);
        let mut diagnostic = tcx.dcx().struct_span_warn(span, TITLE);
        diagnostic.note(warning_message);
        diagnostic.emit();
    }
}
