//! The domain operations. Every adapter translates into exactly these.

mod create_entry;
mod create_folder;
mod create_vault;
mod delete_path;
mod frontmatter;
mod link_graph;
mod links;
mod list_documents;
mod list_entries;
mod list_memories;
mod move_path;
mod read_documents;
mod read_entry;
mod read_memory;
mod read_range;
mod relink;
mod resolve;
mod types;
mod write_document;
mod write_entry;
mod write_memory;

pub use create_entry::{create_entry, CreateEntryRequest, CreateEntryResponse};
pub use create_folder::{create_folder, CreateFolderRequest, CreateFolderResponse};
pub use create_vault::{create_vault, CreateMode, CreateVaultRequest, CreateVaultResponse};
pub use delete_path::{delete_path, DeletePathRequest, DeletePathResponse};
pub use link_graph::{
    link_graph, GraphEdge, GraphNode, GraphTruncation, LinkGraphRequest, LinkGraphResponse,
    UnresolvedLink,
};
pub use list_documents::{list_documents, DocumentEntry, ListDocumentsRequest, ListDocumentsResponse};
pub use list_entries::{list_entries, EntryMeta, ListEntriesRequest, ListEntriesResponse};
pub use list_memories::{list_memories, ListMemoriesRequest, ListMemoriesResponse, MemoryEntry};
pub use move_path::{move_path, MovePathRequest, MovePathResponse};
pub use read_documents::{
    read_documents, DocumentRead, DocumentSelection, ReadDocumentsRequest, ReadDocumentsResponse,
    SkipReason,
};
pub use read_entry::{read_entry, ReadEntryRequest};
pub use relink::{
    relink, RelinkRequest, RelinkResponse, RelinkSkip, RelinkTruncation, RelinkUpdate, RelinkSkipReason,
};
pub use read_memory::{read_memory, ReadMemoryRequest};
pub use types::{DocumentKind, EntryKind, MemoryKind, ReadResult};
pub use write_document::{write_document, WriteDocumentRequest, WriteDocumentResponse};
pub use write_entry::{write_entry, WriteEntryRequest, WriteEntryResponse};
pub use write_memory::{write_memory, WriteMemoryRequest, WriteMemoryResponse};
