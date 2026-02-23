//! Parser for `#[roam::service]` trait definitions.
//!
//! Provides unsynn-based grammar types and a [`parse_service`] entry point.
//! The consumer (`roam-macros`) uses the returned [`ServiceTrait`] to drive
//! code generation.

use proc_macro2::{Ident, TokenStream};
use quote::ToTokens;
use unsynn::*;

// ============================================================================
// OPERATORS AND KEYWORDS
// ============================================================================

operator! {
    /// `;`
    pub Semi = ";";
    /// `&`
    pub Amp = "&";
    /// `::`
    pub PathSep = "::";
    /// `=`
    pub Eq = "=";
}

keyword! {
    /// `pub`
    pub KPub = "pub";
    /// `async`
    pub KAsync = "async";
    /// `fn`
    pub KFn = "fn";
    /// `trait`
    pub KTrait = "trait";
    /// `self`
    pub KSelf = "self";
    /// `mut`
    pub KMut = "mut";
    /// `doc` (as used inside `#[doc = "..."]` attributes)
    pub KDoc = "doc";
    /// `Result` — used to detect fallible return types
    pub KResult = "Result";
    /// `in` — used in `pub(in path)`
    pub KIn = "in";
}

// ============================================================================
// ANGLE-AWARE TOKEN TREE
// ============================================================================

unsynn! {
    /// A single token or a balanced `<...>` group.
    ///
    /// Angle brackets are not proc-macro `Group`s, so we handle them manually.
    #[derive(Clone)]
    pub struct AngleTokenTree(
        pub Either<Cons<Lt, Vec<Cons<Except<Gt>, AngleTokenTree>>, Gt>, TokenTree>,
    );
}

/// Zero or more tokens, stopping before an instance of `C` at the current level.
/// Angle brackets are consumed as a unit so nested `<T, E>` are not terminated
/// early by commas or `>` inside them.
pub type VerbatimUntil<C> = Many<Cons<Except<C>, AngleTokenTree>>;

// ============================================================================
// GRAMMAR
// ============================================================================

unsynn! {
    /// A `#[roam::service]`-annotated trait definition.
    pub struct ServiceTrait {
        /// Attributes on the trait (including any that weren't `#[roam::service]`).
        pub attributes: Vec<Attribute>,
        /// Optional visibility modifier.
        pub vis: Option<Vis>,
        pub _trait_kw: KTrait,
        /// The service name (e.g. `Adder`).
        pub name: Ident,
        /// Body containing zero or more method declarations.
        pub body: BraceGroupContaining<Vec<ServiceMethod>>,
    }

    /// Visibility modifier: `pub`, `pub(crate)`, `pub(in path)`, etc.
    pub enum Vis {
        /// `pub(...)` — parenthesised restriction
        PubGroup(Cons<KPub, ParenthesisGroup>),
        /// bare `pub`
        Pub(KPub),
    }

    /// An outer attribute `#[...]`.
    pub struct Attribute {
        pub _pound: Pound,
        pub body: BracketGroupContaining<AttributeInner>,
    }

    /// The content of an attribute bracket.
    pub enum AttributeInner {
        /// `doc = "..."` — a doc comment.
        Doc(DocAttr),
        /// Anything else (forwarded verbatim).
        Any(Vec<TokenTree>),
    }

    /// `doc = "some text"` inside `#[doc = "some text"]`.
    pub struct DocAttr {
        pub _kw: KDoc,
        pub _eq: Eq,
        /// The doc string value (including quotes in the raw token).
        pub value: LiteralString,
    }

    /// A single `async fn` declaration in the service trait body.
    pub struct ServiceMethod {
        /// Attributes on this method.
        pub attributes: Vec<Attribute>,
        pub _async_kw: KAsync,
        pub _fn_kw: KFn,
        /// Method name.
        pub name: Ident,
        /// Parameter list, including `&self`.
        pub params: ParenthesisGroupContaining<MethodParams>,
        /// Return type after `->`, if present.  Absent means `()`.
        pub return_ty: Option<MethodReturnType>,
        pub _semi: Semi,
    }

    /// The contents of the parameter list `(...)`.
    pub struct MethodParams {
        pub items: CommaDelimitedVec<MethodParam>,
    }

    /// A single parameter — either the receiver (`&self`) or a named argument.
    pub enum MethodParam {
        /// `& (mut)? self`
        Self_(SelfParam),
        /// `name: Type`
        Named(NamedParam),
    }

    /// The `&self` (or `&mut self`) receiver.
    pub struct SelfParam {
        pub _amp: Amp,
        pub _mut: Option<KMut>,
        pub _self: KSelf,
    }

    /// A named parameter `ident: Type`.
    pub struct NamedParam {
        /// Argument name.
        pub name: Ident,
        pub _colon: Colon,
        /// Argument type — everything up to the next `,` at this nesting level.
        pub ty: VerbatimUntil<Comma>,
    }

    /// `-> ReturnType` on a method.
    pub struct MethodReturnType {
        pub _arrow: RArrow,
        /// Parsed return type, with `Result<T, E>` distinguished from plain types.
        pub kind: ReturnTyKind,
    }

    /// Either a `Result<Ok, Err>` return or any other type.
    pub enum ReturnTyKind {
        /// `Result<T, E>` — fallible method.
        Result(ResultTy),
        /// Anything else — infallible method (includes `()`).
        Plain(PlainTy),
    }

    /// `Result<OkType, ErrType>`.
    pub struct ResultTy {
        pub _kw: KResult,
        pub _lt: Lt,
        /// The `Ok` type.
        pub ok: VerbatimUntil<Comma>,
        pub _comma: Comma,
        /// The `Err` type.
        pub err: VerbatimUntil<Gt>,
        pub _gt: Gt,
    }

    /// A plain (infallible) return type — everything up to the trailing `;`.
    pub struct PlainTy {
        pub tokens: VerbatimUntil<Semi>,
    }
}

// ============================================================================
// HELPERS
// ============================================================================

/// Extract doc strings from a slice of [`Attribute`]s.
///
/// Returns one `String` per `#[doc = "..."]` attribute, in order.
/// The strings are the raw literal values (no surrounding quotes, no leading space stripping).
pub fn extract_doc(attributes: &[Attribute]) -> Vec<String> {
    attributes
        .iter()
        .filter_map(|attr| match &attr.body.content {
            AttributeInner::Doc(d) => Some(d.value.to_string()),
            _ => None,
        })
        .collect()
}

/// Convert a [`VerbatimUntil`] capture to a [`TokenStream`].
pub fn verbatim_to_ts<C>(v: &VerbatimUntil<C>) -> TokenStream {
    let mut ts = TokenStream::new();
    for item in v.iter() {
        // Each item is Cons<Except<C>, AngleTokenTree>; the second field carries the token.
        item.value.second.to_tokens(&mut ts);
    }
    ts
}

/// Emit the visibility modifier as a [`TokenStream`].
pub fn vis_to_ts(vis: &Option<Vis>) -> TokenStream {
    match vis {
        Some(Vis::Pub(kw)) => kw.to_token_stream(),
        Some(Vis::PubGroup(cons)) => {
            let mut ts = TokenStream::new();
            cons.first.to_tokens(&mut ts);
            cons.second.to_tokens(&mut ts);
            ts
        }
        None => TokenStream::new(),
    }
}

// ============================================================================
// ENTRY POINT
// ============================================================================

/// Parse a `#[roam::service]` trait definition.
///
/// The `input` token stream should be the item the attribute was applied to
/// (i.e. the trait body), as passed to the `#[proc_macro_attribute]` handler.
pub fn parse_service(input: TokenStream) -> Result<ServiceTrait, unsynn::Error> {
    let mut iter = input.to_token_iter();
    iter.parse()
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn parse_simple_infallible() {
        let input = quote! {
            pub trait Adder {
                /// Add two numbers.
                async fn add(&self, l: u32, r: u32) -> u32;
            }
        };

        let svc = parse_service(input).expect("parse failed");
        assert_eq!(svc.name.to_string(), "Adder");
        assert!(matches!(svc.vis, Some(Vis::Pub(_))));

        let methods: Vec<_> = svc.body.content.iter().map(|m| &m.value).collect();
        assert_eq!(methods.len(), 1);

        let add = methods[0];
        assert_eq!(add.name.to_string(), "add");

        let doc = extract_doc(&add.attributes);
        assert_eq!(doc, vec![" Add two numbers."]);

        let named: Vec<_> = add
            .params
            .content
            .items
            .iter()
            .filter_map(|p| match &p.value {
                MethodParam::Named(n) => Some(n),
                _ => None,
            })
            .collect();

        assert_eq!(named.len(), 2);
        assert_eq!(named[0].name.to_string(), "l");
        assert_eq!(verbatim_to_ts(&named[0].ty).to_string(), "u32");
        assert_eq!(named[1].name.to_string(), "r");
        assert_eq!(verbatim_to_ts(&named[1].ty).to_string(), "u32");

        assert!(matches!(
            add.return_ty.as_ref().unwrap().kind,
            ReturnTyKind::Plain(_)
        ));
    }

    #[test]
    fn parse_fallible() {
        let input = quote! {
            trait Calculator {
                async fn divide(&self, a: f64, b: f64) -> Result<f64, DivideError>;
            }
        };

        let svc = parse_service(input).expect("parse failed");
        let method = &svc.body.content[0].value;

        match &method.return_ty.as_ref().unwrap().kind {
            ReturnTyKind::Result(r) => {
                assert_eq!(verbatim_to_ts(&r.ok).to_string(), "f64");
                assert_eq!(verbatim_to_ts(&r.err).to_string(), "DivideError");
            }
            _ => panic!("expected Result return type"),
        }
    }

    #[test]
    fn parse_unit_return() {
        let input = quote! {
            trait Notifier {
                async fn notify(&self, msg: String);
            }
        };

        let svc = parse_service(input).expect("parse failed");
        let method = &svc.body.content[0].value;
        assert!(method.return_ty.is_none());
    }

    #[test]
    fn parse_generic_arg() {
        let input = quote! {
            trait Streamer {
                async fn stream(&self, items: Vec<String>) -> Option<u64>;
            }
        };

        let svc = parse_service(input).expect("parse failed");
        let method = &svc.body.content[0].value;

        let named: Vec<_> = method
            .params
            .content
            .items
            .iter()
            .filter_map(|p| match &p.value {
                MethodParam::Named(n) => Some(n),
                _ => None,
            })
            .collect();

        assert_eq!(named.len(), 1);
        assert_eq!(named[0].name.to_string(), "items");
        let ty = verbatim_to_ts(&named[0].ty).to_string();
        assert_eq!(ty, "Vec < String >");

        match &method.return_ty.as_ref().unwrap().kind {
            ReturnTyKind::Plain(p) => {
                assert_eq!(verbatim_to_ts(&p.tokens).to_string(), "Option < u64 >");
            }
            _ => panic!("expected plain return type"),
        }
    }

    #[test]
    fn parse_multiple_methods() {
        let input = quote! {
            pub trait Echo {
                async fn ping(&self) -> u64;
                async fn echo(&self, msg: String) -> String;
                async fn close(&self);
            }
        };

        let svc = parse_service(input).expect("parse failed");
        assert_eq!(svc.body.content.len(), 3);
        assert_eq!(svc.body.content[0].value.name.to_string(), "ping");
        assert_eq!(svc.body.content[1].value.name.to_string(), "echo");
        assert_eq!(svc.body.content[2].value.name.to_string(), "close");
    }
}
