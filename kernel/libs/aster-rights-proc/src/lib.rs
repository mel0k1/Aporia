use require_item::RequireItem;
use syn::parse_macro_input;

use crate::require_attr::{RequireAttr, expand_require};

mod require_attr;
mod require_item;

#[proc_macro_attribute]
pub fn require(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let require_item = parse_macro_input!(item as RequireItem);
    let require_attr = parse_macro_input!(attr as RequireAttr);
    expand_require(require_item, require_attr).into()
}
