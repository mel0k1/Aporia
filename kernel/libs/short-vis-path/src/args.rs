use std::collections::BTreeMap;

use proc_macro2::{Group, Span, TokenStream, TokenTree};
use quote::ToTokens;
use syn::{parse::Parse, punctuated::Punctuated, *};

enum Argument {
    Single(Ident),
    Override(Ident, Path),
}

impl Parse for Argument {
    fn parse(input: parse::ParseStream) -> Result<Self> {
        Ok(if input.peek2(Token![=]) {
            let ident: Ident = input.parse()?;
            let _: Token![=] = input.parse()?;
            let path: Path = input.parse()?;
            Argument::Override(ident, path)
        } else {
            let ident: Ident = input.parse()?;
            Argument::Single(ident)
        })
    }
}

pub(crate) struct AddArguments {
    args: BTreeMap<Ident, Path>,
}

impl Parse for AddArguments {
    fn parse(input: parse::ParseStream) -> Result<Self> {
        let args = Punctuated::<Argument, Token![,]>::parse_terminated(input)?;

        let path = ExpandedPath::new();

        Ok(AddArguments {
            args: args
                .into_iter()
                .map(|arg| match arg {
                    Argument::Single(ident) => {
                        let Some(tokens) = path.to_syn_path(&ident) else {
                            panic!(
                                "The path `{}` doesn't contain `{ident}`. \
                                 Please choose a correct short module name.",
                                path.segment.join("::")
                            )
                        };
                        (ident, tokens)
                    }
                    Argument::Override(ident, path) => (ident, path.clone()),
                })
                .collect(),
        })
    }
}

impl visit_mut::VisitMut for AddArguments {
    fn visit_visibility_mut(&mut self, vis: &mut Visibility) {
        self.replace_restricted_vis_path(vis);
    }

    fn visit_item_mut(&mut self, item: &mut Item) {
        if let Item::Verbatim(ts) = item {
            self.replace_verbatim_vis_path(ts);
            return;
        }
        visit_mut::visit_item_mut(self, item);
    }
}

impl AddArguments {
    fn replace_restricted_vis_path(&self, vis: &mut Visibility) {
        if let Visibility::Restricted(vis) = vis
            && let Some(input) = vis.path.get_ident()
            && let Some(path) = self.args.get(input)
        {
            vis.path = Box::clone_from_ref(path);
        }
    }

    fn replace_verbatim_vis_path(&self, ts: &mut TokenStream) {
        let mut v_tt: Vec<TokenTree> = ts.clone().into_iter().collect();
        let mut iter = v_tt.iter_mut();
        if let Some(TokenTree::Ident(ident)) = iter.next()
            && ident == "pub"
            && let Some(TokenTree::Group(group)) = iter.next()
        {
            let mut new_stream = TokenStream::new();
            let mut stream = group.stream().into_iter();
            if let Some(in_) = stream.next()
                && let TokenTree::Ident(ident) = &in_
                && ident == "in"
            {
                new_stream.extend([in_]);

                let path_stream = stream.collect::<TokenStream>();
                if let Ok(input) = parse2::<Ident>(path_stream)
                    && let Some(path) = self.args.get(&input)
                {
                    path.to_tokens(&mut new_stream);
                    *group = Group::new(group.delimiter(), new_stream);
                }
            }
            *ts = TokenStream::from_iter(v_tt);
        }
    }

    #[cfg(test)]
    pub(crate) fn test_new(ident: &str, path: &str) -> Self {
        let mut args = BTreeMap::new();
        let path = parse_str(path).unwrap();
        args.insert(Ident::new(ident, Span::call_site()), path);
        AddArguments { args }
    }
}

struct ExpandedPath {
    segment: Vec<String>,
    callsite_span: Span,
}

impl ExpandedPath {
    fn new() -> Self {
        let callsite_span = Span::call_site();
        let Some(local_path) = callsite_span.local_file() else {
            panic!("Unknown local file path to call site span {callsite_span:?}.");
        };
        let Ok(local_path) = local_path.canonicalize() else {
            panic!("Unable to canonicalize {local_path:?}.")
        };

        let prefix = {
            let dir = std::env::var("CARGO_MANIFEST_DIR").expect("Failed to get manifest dir.");
            std::path::PathBuf::from(dir).join("src")
        };

        let Ok(module_path) = local_path.strip_prefix(&prefix) else {
            panic!("{prefix:?} must be a prefix of {local_path:?}.")
        };

        let module_path = if module_path.file_name() == Some("mod.rs".as_ref()) {
            module_path.parent().unwrap()
        } else {
            module_path
        };
        let module_path = module_path.with_extension("");

        ExpandedPath {
            segment: std::iter::once("crate")
                .chain(module_path.iter().map(|m| m.to_str().unwrap()))
                .map(String::from)
                .collect(),
            callsite_span,
        }
    }

    fn to_syn_path(&self, end: &Ident) -> Option<Path> {
        let pos = self.segment.iter().rposition(|seg| end == seg.as_str())?;
        Some(Path {
            leading_colon: None,
            segments: self.segment[..pos + 1]
                .iter()
                .map(|s| PathSegment::from(Ident::new(s, self.callsite_span)))
                .collect(),
        })
    }
}
