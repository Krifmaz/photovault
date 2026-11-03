//! Public links (albums, single files, memories) and cast: authentication,
//! storage of this workstream's tables, and the file access they share.
//!
//! Everything that touches tables owned by the files/collections workstream goes
//! through [`library_sql`].

pub mod cast;
pub mod collect;
pub mod files;
pub mod http;
pub mod jwt;
pub mod library_sql;
pub mod links;
pub mod memory;
pub mod models;
