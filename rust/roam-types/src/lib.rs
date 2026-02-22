macro_rules! declare_id {
    ($(#[$meta:meta])* $name:ident, $inner:ty) => {
        $(#[$meta])*
        #[derive(Facet, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Clone, Copy)]
        #[repr(transparent)]
        #[facet(transparent)]
        pub struct $name(pub $inner);

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

mod message;
pub use message::*;

mod connectivity;
pub use connectivity::*;

mod metadata;
pub use metadata::*;
