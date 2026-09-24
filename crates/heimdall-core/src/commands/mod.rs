//! The domain operations. Every adapter translates into exactly these.

mod create_folder;
mod create_vault;
mod delete_path;
mod link_graph;
mod links;
mod listing;
mod lock;
mod move_path;
mod read;
mod read_range;
mod relink;
mod resolve;
mod types;
mod write;

pub use create_folder::{create_folder, CreateFolderRequest, CreateFolderResponse};
pub use create_vault::{create_vault, CreateMode, CreateVaultRequest, CreateVaultResponse};
pub use delete_path::{delete_path, DeletePathRequest, DeletePathResponse};
pub use link_graph::{
    link_graph, GraphEdge, GraphNode, GraphTruncation, LinkGraphRequest, LinkGraphResponse,
    UnresolvedLink,
};
pub use listing::{DocumentEntry, Listing};
pub use lock::{lock, unlock, LockRequest, LockResponse};
pub use move_path::{move_path, MovePathRequest, MovePathResponse};
pub use read::{read, ReadRequest, ReadResponse};
pub use relink::{
    relink, RelinkRequest, RelinkResponse, RelinkSkip, RelinkSkipReason, RelinkTruncation,
    RelinkUpdate,
};
pub use types::{DocumentKind, ReadResult};
pub use write::{write, WriteRequest, WriteResponse};
