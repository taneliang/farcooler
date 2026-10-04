//! Every method on the wire has a handler in `Rpc::dispatch`, and every
//! handler is for a method on the wire.
//!
//! A SET DIFFERENCE, both ways, because the two sides fail differently. An arm
//! that is not a `Method` is unreachable: `required_scope` refuses it as an
//! unknown method, which reads to a client as "this runner is too old" for a
//! feature this runner has. That is how the agent queue's three shipped. A
//! `Method` with no arm is worse: the scope check passes, the method falls
//! through to the last arm, and the caller is told `NotFound` as though the
//! thing it named did not exist.
//!
//! Scopes need no such check: `scope_of` is a match on `Method` with no
//! wildcard, so the compiler holds that side. Dispatch matches wire names and
//! the compiler cannot.
//!
//! So `rpc.rs` is parsed (ov-171) and the patterns of the arms are read off the
//! syntax tree: a name in a log line, a comment or a string argument is not an
//! arm. The check was a scan of the source text for string literals, which
//! counted all of those, so a method could be "dispatched" with its arm
//! deleted and a mention of it left behind.

use std::collections::BTreeSet;

use farcooler_protocol::method::Method;
use syn::visit::Visit;
use syn::{Expr, ExprMatch, Lit, Pat};

const RPC: &str = include_str!("../src/rpc.rs");

/// The string literals in an arm's pattern: `"a"`, or `"a" | "b"`.
fn literals(pat: &Pat, into: &mut BTreeSet<String>) {
    match pat {
        Pat::Lit(lit) => {
            if let Lit::Str(s) = &lit.lit {
                into.insert(s.value());
            }
        }
        Pat::Or(or) => or.cases.iter().for_each(|case| literals(case, into)),
        _ => {}
    }
}

/// Whether a match is on the method's name: `req.method.as_str()` or `method`.
fn matches_on_the_method(scrutinee: &Expr) -> bool {
    match scrutinee {
        Expr::MethodCall(call) if call.method == "as_str" => matches_on_the_method(&call.receiver),
        Expr::Field(field) => matches!(&field.member, syn::Member::Named(n) if n == "method"),
        Expr::Path(path) => path.path.is_ident("method"),
        _ => false,
    }
}

#[derive(Default)]
struct Arms {
    routes: BTreeSet<String>,
    matches: usize,
}

impl<'ast> Visit<'ast> for Arms {
    fn visit_expr_match(&mut self, node: &'ast ExprMatch) {
        if matches_on_the_method(&node.expr) {
            self.matches += 1;
            for arm in &node.arms {
                literals(&arm.pat, &mut self.routes);
            }
        }
        syn::visit::visit_expr_match(self, node);
    }
}

/// The wire names `Rpc::dispatch` has an arm for, read off the parsed file.
fn dispatched() -> (BTreeSet<String>, usize) {
    struct Find(Option<Arms>);
    impl<'ast> Visit<'ast> for Find {
        fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
            if node.sig.ident == "dispatch" && node.sig.asyncness.is_some() {
                let mut arms = Arms::default();
                arms.visit_block(&node.block);
                self.0 = Some(arms);
            }
            syn::visit::visit_impl_item_fn(self, node);
        }
    }
    let file = syn::parse_file(RPC).expect("rpc.rs parses");
    let mut find = Find(None);
    find.visit_file(&file);
    let arms = find.0.expect("rpc.rs declares `async fn dispatch` in an impl");
    (arms.routes, arms.matches)
}

#[test]
fn every_method_is_dispatched_and_every_dispatched_route_is_a_method() {
    let (dispatched, matches) = dispatched();
    let methods: BTreeSet<String> = Method::ALL.iter().map(|m| m.name().to_string()).collect();

    assert!(
        dispatched.len() > 50 && matches >= 2,
        "the parse found {} routes in {matches} matches, so this test proves nothing",
        dispatched.len()
    );
    let unknown: Vec<_> = dispatched.difference(&methods).collect();
    assert!(unknown.is_empty(), "dispatched but refused as unknown on every runner: {unknown:?}");
    let unhandled: Vec<_> = methods.difference(&dispatched).collect();
    assert!(unhandled.is_empty(), "a method with a scope and no handler: {unhandled:?}");
}
