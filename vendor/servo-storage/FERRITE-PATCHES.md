# Ferrite's changes to servo-storage 0.6.0

This directory is `servo-storage` 0.6.0 from crates.io (MPL-2.0, see the headers of
its files) with the changes below. It is used through `[patch.crates-io]` in the
workspace `Cargo.toml`, together with `vendor/servo-storage-traits` (the message types
it shares with the engine). Drop both when the engine is upgraded to a release that
enforces unique indexes.

1. `indexeddb/engines/sqlite.rs`, `indexeddb/engines/sqlite/create.rs`: **unique
   indexes are enforced.** The backend cannot read a stored value, so the script thread
   sends, with every `PutItem`, the keys the record has in each unique index of its store
   (`unique_index_keys`). `put_item` checks them against the `unique_index_data` table
   (which the schema had and nothing used) before writing anything, so a refused put
   leaves the store as it was, and answers `PutItemResult::UniqueIndexViolation` (the
   script side raises `ConstraintError`). The record's old keys in that table are
   replaced when it is saved again, and removed when the record is deleted
   (`delete_item`), the store is cleared (`clear`) or the index is deleted
   (`delete_index`). Test: `test_unique_index_is_enforced`.
   **Not done:** a unique index made *after* records exist (`createIndex` in a later
   version upgrade) is not checked against them, and does not abort the upgrade when
   they already hold a duplicate; only records written after the index exists are
   checked.
2. `indexeddb/engines/sqlite/create.rs`: the `object_store_index` table made the index
   *name* unique across the whole database, so two object stores could not each have
   an index called `by_date` (the second `createIndex` failed). It is unique per object
   store now. A database created before this change keeps the old table.
