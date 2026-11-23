use syn::parse_macro_input;

use crate::{type_flag::TypeFlagDef, util::expand_type_flag};

mod flag_set;
mod type_flag;
mod util;

#[proc_macro]
pub fn typeflags(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let type_flags_def = parse_macro_input!(input as TypeFlagDef);
    expand_type_flag(&type_flags_def).into()
}
