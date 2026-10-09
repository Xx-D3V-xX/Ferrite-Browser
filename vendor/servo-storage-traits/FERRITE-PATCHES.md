# Ferrite's changes to servo-storage-traits 0.6.0

`servo-storage-traits` 0.6.0 from crates.io (MPL-2.0) with the changes below, used
through `[patch.crates-io]` together with `vendor/servo-storage`.

1. `indexeddb.rs`: `AsyncReadWriteOperation::PutItem` carries `unique_index_keys`, and
   `PutItemResult` has a `UniqueIndexViolation` variant. See
   `vendor/servo-storage/FERRITE-PATCHES.md`, item 1.
