macro_rules! declare_u64_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Facet, PartialEq, Eq, Hash, Debug, Clone, Copy)]
        #[repr(transparent)]
        #[facet(transparent)]
        pub struct $name(pub u64);

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

mod rpc_plan;
pub use rpc_plan::*;

mod services;
pub use services::*;

mod requests;
pub use requests::*;

mod wire;
pub use wire::*;
