use syn::{ItemFn, ItemImpl, Token, parse::Parse};

pub(crate) enum RequireItem {
    Impl(ItemImpl),
    Fn(ItemFn),
}

impl Parse for RequireItem {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let token = input.lookahead1();
        if token.peek(Token!(impl)) {
            input.parse().map(RequireItem::Impl)
        } else {
            input.parse().map(RequireItem::Fn)
        }
    }
}
