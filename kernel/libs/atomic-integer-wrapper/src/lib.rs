#![feature(proc_macro_diagnostic)]

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{quote, quote_spanned};
use syn::{
    Error, Fields, Ident, ItemStruct, LitBool, Result, Token, Type, braced,
    parse::{Parse, ParseStream},
    parse_macro_input,
    spanned::Spanned,
};

struct Input {
    integer_like_type: Type,
    try_from: LitBool,
    item: ItemStruct,
}

impl Parse for Input {
    fn parse(input: ParseStream) -> Result<Self> {
        let integer_like_type: Type = input.parse()?;
        input.parse::<Token![,]>()?;

        let mut try_from = LitBool::new(false, Span::call_site());
        let lookahead = input.lookahead1();
        if lookahead.peek(Ident) {
            let key = input.parse::<Ident>()?;
            if key != "try_from" {
                return Err(Error::new(
                    key.span(),
                    format!(r#"Expected "try_from", found "{}""#, key),
                ));
            }
            input.parse::<Token![=]>()?;
            try_from = input.parse()?;
            input.parse::<Token![,]>()?;
        }

        let content;
        braced!(content in input);
        let item: ItemStruct = content.parse()?;

        if !input.is_empty() {
            return Err(Error::new(Span::call_site(), "Unexpected token"));
        }

        Ok(Input {
            integer_like_type,
            try_from,
            item,
        })
    }
}

#[proc_macro]
pub fn define_atomic_version_of_integer_like_type(input: TokenStream) -> TokenStream {
    let Input {
        integer_like_type,
        try_from,
        item,
    } = parse_macro_input!(input as Input);

    let atomic_wrapper = item.ident.clone();
    let atomic_integer_type = if let Fields::Unnamed(ref fields_unnamed) = item.fields
        && fields_unnamed.unnamed.len() == 1
    {
        fields_unnamed.unnamed.first().unwrap().ty.clone()
    } else {
        item.fields
            .span()
            .unwrap()
            .error("Expected a parenthesized struct like `struct AtomicFoo(AtomicU8)`")
            .emit();
        return TokenStream::new();
    };
    let from_integer = if try_from.value {
        quote_spanned! {integer_like_type.span()=>
            try_into().unwrap()
        }
    } else {
        quote_spanned! {integer_like_type.span()=>
            into()
        }
    };

    let fn_new = quote! {
        pub fn new(value: impl Into<#integer_like_type>) -> Self {
            Self(<#atomic_integer_type>::new(value.into().into()))
        }
    };
    let fn_load = quote! {
        pub fn load(&self, order: core::sync::atomic::Ordering) -> #integer_like_type {
            self.0.load(order).#from_integer
        }
    };
    let fn_store = quote! {
        pub fn store(
            &self,
            val: impl Into<#integer_like_type>,
            order: core::sync::atomic::Ordering
        ) {
            self.0.store(val.into().into(), order);
        }
    };
    let fn_swap = quote! {
        pub fn swap(
            &self,
            val: impl Into<#integer_like_type>,
            order: core::sync::atomic::Ordering
        ) -> #integer_like_type {
            self.0.swap(val.into().into(), order).#from_integer
        }
    };
    let fn_compare_exchange = quote! {
        pub fn compare_exchange(
            &self,
            current: impl Into<#integer_like_type>,
            new: impl Into<#integer_like_type>,
            success: core::sync::atomic::Ordering,
            failure: core::sync::atomic::Ordering
        ) -> core::result::Result<#integer_like_type, #integer_like_type> {
            self.0
                .compare_exchange(
                    current.into().into(),
                    new.into().into(),
                    success,
                    failure
                )
                .map(|val| val.#from_integer)
                .map_err(|val| val.#from_integer)
        }
    };
    let fn_try_update = quote! {
        pub fn try_update<F>(
            &self,
            set_order: core::sync::atomic::Ordering,
            fetch_order: core::sync::atomic::Ordering,
            mut f: F
        ) -> core::result::Result<#integer_like_type, #integer_like_type>
        where
            F: FnMut(#integer_like_type) -> Option<#integer_like_type>,
        {
            self.0
                .try_update(
                    set_order,
                    fetch_order,
                    |old| f(old.#from_integer).map(<#integer_like_type>::into)
                )
                .map(|val| val.#from_integer)
                .map_err(|val| val.#from_integer)
        }
    };
    let fn_update = quote! {
        pub fn update<F>(
            &self,
            set_order: core::sync::atomic::Ordering,
            fetch_order: core::sync::atomic::Ordering,
            mut f: F
        ) -> #integer_like_type
        where
            F: FnMut(#integer_like_type) -> #integer_like_type,
        {
            self.0
                .update(
                    set_order,
                    fetch_order,
                    |old| f(old.#from_integer).into()
                )
                .#from_integer
        }
    };

    let expanded = quote! {
        #item

        impl #atomic_wrapper {
            #fn_new

            #fn_load

            #fn_store

            #fn_swap

            #fn_compare_exchange

            #fn_update

            #fn_try_update
        }
    };

    TokenStream::from(expanded)
}
