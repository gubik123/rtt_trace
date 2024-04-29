// Spurious clippy warning on the code generated by Darling
#![allow(clippy::manual_unwrap_or_default)]

use darling::ast::NestedMeta;
use darling::FromMeta;
use quote::quote;

mod codegen;

/// This macro should be used if you want to trace a specific execution span.
/// It will emit trace span from it's point of invocation to the end of the
/// block it was invoked in.
///
/// For example:
///
/// ```ignore
/// foo();
/// {
///     bar();
///     trace_here!(comment="Something is happening");  // <- Trace span will be started here.
///     baz();
/// }                                                   // <- Trace span will be ended here.
/// ```
///
/// This macro accepts following parameters:
/// - `comment=S` --- optional string, which will be saved in the metadata for trace interpretation tool
///   to inspect
/// - `noenter` --- entry point of a span will not be emited. Useful to trace events.
/// - `noexit` --- exit point of a stan will not be emited.
/// - `skip=N` --- Report span entry and exit only each Nth time. Can be used to relief the transport
///   bandwidth requirement.
///
#[proc_macro]
pub fn trace_here(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let attrs = NestedMeta::parse_meta_list(input.into()).expect("Malformed trace_here! arguments");
    let attrs = FreestandingMeta::from_list(&attrs).expect("Unable to parse trace_here! arguments");

    let ret = codegen::tracer_instantiation(
        utrace_core::trace_point::TracePointPairKind::Generic,
        None,
        attrs.comment,
        attrs.skip,
        !attrs.noenter,
        !attrs.noexit,
    );

    quote! {#ret;}.into()
}

/// This attribute can be applied to functions and async functions to instrument them.
/// By default, when applied to a function, it will trace function entry and function exit.
/// If applied to `async fn`, it will report creation, drop and poll spans of the respective Future.
///
/// This macro accepts following parameters:
/// - `comment=S` --- optional string, which will be saved in the metadata for trace interpretation tool
///   to inspect
/// - `noenter_fn` --- in case used on sync fn, it will disable function entry tracing. If instrumented
///   fn is `async`, Future creation will not be reported.
/// - `noexit_fn` --- same as `noenter_fn`, but exit or Future drop will not be reported. *This will break
///   the current mechanism of Future lifetime tracing*
/// - `noenter_poll` --- applicable to `async fn`. Disables tracing of Future `poll(..)` entry.
/// - `noexit_poll` --- same as `noenter_poll`, but for `poll` exit.
/// - `skip=N` --- Report function entry and exit (or Future creation/drop) only each Nth time. Can
///   be used to releif the transport bandwidth requirement.
/// - `skip_poll=N` --- same, but for poll function.
///
/// <div class="warning">
/// Please note, that current implementation handles skip and skip_poll independent, so specifying
/// skip will likely break Future lifetime tracing.
/// </div>
///
/// Example:
/// ```ignore
/// #[utrace::trace(comment="This is my comment", noenter_fn, noexit_poll)]
/// async fn my_future() {
///     ....
/// }
/// ```
#[proc_macro_attribute]
pub fn trace(
    attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let ast: syn::ItemFn = syn::parse(input).expect("Failed to parse input as a function");

    let attrs = NestedMeta::parse_meta_list(attr.into()).expect("Malformed attr list");
    let attrs =
        FnAttributesMeta::from_list(&attrs).expect("Unable to parse #[trace] attribute arguments");

    let head_ident = &ast.sig;
    let fn_vis = &ast.vis;
    let body = &ast.block;
    let body = if ast.sig.asyncness.is_some() {
        codegen::transform_async_fn(Some(ast.sig.ident.to_string()), attrs, quote! {#body})
    } else if attrs.skip_poll.is_some() || attrs.noenter_poll || attrs.noexit_poll {
        quote! {
            compile_error!("Attributes skip_poll, noenter_poll and noexit_poll cannot be applied to non-async functions");
        }
    } else {
        codegen::transform_sync_fn(Some(ast.sig.ident.to_string()), attrs, quote! {#body})
    };

    let expanded = quote! {
        #fn_vis #head_ident {
            #body
        }
    };

    expanded.into()
}

/// This macro provides a transport implementation for utrace.
///
/// To create custom transport, one should do the following:
///
/// ```ignore
/// #[utrace::default_transport]
/// fn transport(buf: &[u8]) {
///    ...
/// }
/// ```
///
/// Please note, that current implementation executes timestamp capture, serialization and
/// sending in a single critical section, hence the transport function does not need to be
/// reentrant.
///
/// Note, that during initialization you might want to call utrace::init().
/// While this is not mandatory, this will provide a trace stream receiver with a point to
/// synchronize time and split traces related to different runs.
#[proc_macro_attribute]
pub fn default_transport(
    _attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let body: syn::ItemFn =
        syn::parse(input).expect("#[utrace::default_transport] should be applied to a function");

    quote! {
        #[export_name = "__utrace_default_transport_write"]
        #body
    }
    .into()
}

/// This macro should be used to define timestamp function which will be used by utrace
/// to obtain event timestamps.
///
/// For example:
///
/// ```ignore
/// #[utrace::timestamp]
/// fn timestamp() -> u64 {
///     ...
/// }
/// ```
///
/// It should only be invoked in binaries and only once.
#[proc_macro_attribute]
pub fn timestamp(
    _attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let body: syn::ItemFn =
        syn::parse(input).expect("#[utrace::timestamp] should be applied to a function");

    quote! {
        #[export_name = "__utrace_timestamp_function"]
        #body
    }
    .into()
}

#[derive(Debug, FromMeta)]
struct FnAttributesMeta {
    #[darling(default)]
    comment: Option<String>,
    #[darling(default)]
    noenter_fn: bool,
    #[darling(default)]
    noexit_fn: bool,
    #[darling(default)]
    noenter_poll: bool,
    #[darling(default)]
    noexit_poll: bool,
    #[darling(default)]
    skip: Option<u32>,
    #[darling(default)]
    skip_poll: Option<u32>,
}

#[derive(Debug, FromMeta)]
struct FreestandingMeta {
    #[darling(default)]
    comment: Option<String>,
    #[darling(default)]
    noenter: bool,
    #[darling(default)]
    noexit: bool,
    #[darling(default)]
    skip: Option<u32>,
}
