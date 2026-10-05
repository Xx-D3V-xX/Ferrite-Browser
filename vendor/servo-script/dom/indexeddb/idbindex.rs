/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
use dom_struct::dom_struct;
use js::context::JSContext;
use js::conversions::ToJSValConvertible;
use js::gc::MutableHandleValue;
use script_bindings::cell::DomRefCell;
use script_bindings::codegen::GenericBindings::IDBIndexBinding::IDBIndexMethods;
use script_bindings::codegen::GenericBindings::IDBTransactionBinding::IDBTransactionMode;
use js::jsval::UndefinedValue;
use js::rust::HandleValue;
use script_bindings::error::{Error, ErrorResult, Fallible};
use script_bindings::reflector::{Reflector, reflect_dom_object_with_cx};
use script_bindings::str::DOMString;
use storage_traits::indexeddb::{
    AsyncOperation, AsyncReadOnlyOperation, IndexedDBKeyRange, IndexedDBRecord,
};

use crate::dom::bindings::codegen::Bindings::IDBCursorBinding::IDBCursorDirection;
use crate::dom::bindings::refcounted::Trusted;
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::{Dom, DomRoot};
use crate::dom::bindings::structuredclone;
use crate::dom::globalscope::GlobalScope;
use crate::dom::idbobjectstore::KeyPath;
use crate::dom::indexeddb::idbcursor::{IDBCursor, IterationParam, ObjectStoreOrIndex};
use crate::dom::indexeddb::idbcursorwithvalue::IDBCursorWithValue;
use crate::dom::indexeddb::idbobjectstore::IDBObjectStore;
use crate::dom::indexeddb::idbrequest::{
    IDBRequest, IndexQuery, IndexQueryKind, RequestJob, RequestSource,
};
use crate::dom::indexeddb::key::{convert_value_to_key_range, extract_index_keys};

#[dom_struct]
pub(crate) struct IDBIndex {
    reflector_: Reflector,
    object_store: Dom<IDBObjectStore>,
    name: DomRefCell<DOMString>,
    multi_entry: bool,
    unique: bool,
    key_path: KeyPath,
}

impl IDBIndex {
    pub fn new_inherited(
        object_store: &IDBObjectStore,
        name: DOMString,
        multi_entry: bool,
        unique: bool,
        key_path: KeyPath,
    ) -> IDBIndex {
        IDBIndex {
            reflector_: Reflector::new(),
            object_store: Dom::from_ref(object_store),
            name: DomRefCell::new(name),
            multi_entry,
            unique,
            key_path,
        }
    }

    pub fn new(
        cx: &mut JSContext,
        global: &GlobalScope,
        object_store: &IDBObjectStore,
        name: DOMString,
        multi_entry: bool,
        unique: bool,
        key_path: KeyPath,
    ) -> DomRoot<IDBIndex> {
        reflect_dom_object_with_cx(
            Box::new(IDBIndex::new_inherited(
                object_store,
                name,
                multi_entry,
                unique,
                key_path,
            )),
            global,
            cx,
        )
    }

    /// The checks every request method starts with
    /// (<https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-get> steps 3 and 4): the
    /// index and its store are still there, and the transaction is active.
    fn check_usable(&self) -> Fallible<()> {
        if !self.object_store.has_index(&self.name.borrow()) {
            return Err(Error::InvalidState(None));
        }
        self.object_store.verify_not_deleted()?;
        self.object_store.check_transaction_active()
    }

    /// Whether this index (or its store) has been deleted.
    pub(crate) fn is_deleted(&self) -> bool {
        !self.object_store.has_index(&self.name.borrow()) ||
            self.object_store.verify_not_deleted().is_err()
    }

    /// Runs a query on the index. The backend sends every record of the store and
    /// `idbrequest::index_answer` works out the answer from them.
    fn query(
        &self,
        cx: &mut JSContext,
        range: IndexedDBKeyRange,
        kind: IndexQueryKind,
    ) -> Fallible<DomRoot<IDBRequest>> {
        let request = IDBRequest::execute_async(
            cx,
            &self.object_store,
            |callback| {
                AsyncOperation::ReadOnly(AsyncReadOnlyOperation::Iterate {
                    callback,
                    key_range: IndexedDBKeyRange::default(),
                })
            },
            None,
            Some(RequestJob::IndexQuery(IndexQuery {
                index: Trusted::new(self),
                range,
                kind,
            })),
        )?;
        request.set_source(Some(RequestSource::Index(Dom::from_ref(self))));
        Ok(request)
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-opencursor>
    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-openkeycursor>
    fn open_cursor(
        &self,
        cx: &mut JSContext,
        query: HandleValue,
        direction: IDBCursorDirection,
        key_only: bool,
    ) -> Fallible<DomRoot<IDBRequest>> {
        self.check_usable()?;
        let range = convert_value_to_key_range(cx, query, Some(false))?;
        let transaction = self.object_store.transaction();
        let source = ObjectStoreOrIndex::Index(Dom::from_ref(self));
        let cursor = if key_only {
            IDBCursor::new(
                cx,
                &self.global(),
                &transaction,
                direction,
                false,
                source,
                range.clone(),
                key_only,
            )
        } else {
            DomRoot::upcast(IDBCursorWithValue::new(
                cx,
                &self.global(),
                &transaction,
                direction,
                false,
                source,
                range.clone(),
                key_only,
            ))
        };
        let iteration_param = IterationParam {
            cursor: Trusted::new(&cursor),
            key: None,
            primary_key: None,
            count: None,
        };
        // The backend sends the whole store; the cursor's own range selects within
        // the index keys (`iterate_cursor`).
        let request = IDBRequest::execute_async(
            cx,
            &self.object_store,
            |callback| {
                AsyncOperation::ReadOnly(AsyncReadOnlyOperation::Iterate {
                    callback,
                    key_range: IndexedDBKeyRange::default(),
                })
            },
            None,
            Some(RequestJob::Cursor(iteration_param)),
        )?;
        request.set_source(Some(RequestSource::Index(Dom::from_ref(self))));
        cursor.set_request(&request);
        Ok(request)
    }

    pub(crate) fn store(&self) -> DomRoot<IDBObjectStore> {
        self.object_store.as_rooted()
    }

    pub(crate) fn key_path(&self) -> &KeyPath {
        &self.key_path
    }

    pub(crate) fn multi_entry(&self) -> bool {
        self.multi_entry
    }
}

/// The index's records for a store's records: one per (index key, record), in index
/// order (index key, then primary key). A record whose value gives no valid key for
/// the index is not in it. See `extract_index_keys`.
pub(crate) fn index_records(
    cx: &mut JSContext,
    global: &GlobalScope,
    index: &IDBIndex,
    records: Vec<IndexedDBRecord>,
) -> Result<Vec<IndexedDBRecord>, Error> {
    let mut entries: Vec<IndexedDBRecord> = Vec::with_capacity(records.len());
    for record in records {
        rooted!(&in(cx) let mut value = UndefinedValue());
        postcard::from_bytes(&record.value)
            .map_err(|_| Error::Data(None))
            .and_then(|data| structuredclone::read(cx, global, data, value.handle_mut()))?;
        for key in extract_index_keys(cx, value.handle(), &index.key_path, index.multi_entry)? {
            entries.push(IndexedDBRecord {
                key,
                primary_key: record.primary_key.clone(),
                value: record.value.clone(),
            });
        }
    }
    entries.sort_by(|a, b| {
        a.key
            .partial_cmp(&b.key)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.primary_key
                    .partial_cmp(&b.primary_key)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    Ok(entries)
}

impl IDBIndexMethods<crate::DomTypeHolder> for IDBIndex {
    /// <https://www.w3.org/TR/IndexedDB/#dom-idbindex-name>
    fn Name(&self) -> DOMString {
        self.name.borrow().clone()
    }

    /// <https://www.w3.org/TR/IndexedDB/#ref-for-dom-idbindex-name%E2%91%A2>
    fn SetName(&self, name: DOMString) -> ErrorResult {
        // Step 1: Let name be the given value.
        // Step 2: Let transaction be this’s transaction.
        let transaction = self.object_store.transaction();

        // Step 3: Let index be this’s index.
        // We do not have an explicit object representing the underlying index.

        // Step 4: If transaction is not an upgrade transaction, throw an "InvalidStateError" DOMException.
        if transaction.get_mode() != IDBTransactionMode::Versionchange {
            return Err(Error::InvalidState(Some(
                "Transaction is not an upgrade transaction".to_owned(),
            )));
        }

        // Step 5: If transaction’s state is not active, then throw a "TransactionInactiveError" DOMException.
        if !transaction.is_active() {
            return Err(Error::TransactionInactive(Some(
                "Transaction is not active while updating index name".to_owned(),
            )));
        }

        // Step 6: If index or index’s object store has been deleted, throw an "InvalidStateError" DOMException.
        let mut stored_name = self.name.borrow_mut();
        if !self.object_store.has_index(&stored_name) ||
            !transaction
                .get_db()
                .object_store_exists(&self.object_store.get_name())
        {
            return Err(Error::InvalidState(Some(
                "Index or its object store has been deleted".to_owned(),
            )));
        }

        // Step 7: If index’s name is equal to name, terminate these steps.
        if *stored_name == name {
            return Ok(());
        }

        // Step 8: If an index named name already exists in index’s object store, throw a "ConstraintError" DOMException.
        if self.object_store.has_index(&name) {
            return Err(Error::Constraint(Some(
                "An index with the given name already exists".to_owned(),
            )));
        }

        // Step 9: Set index’s name to name.
        self.object_store.rename_index(&stored_name, &name);

        // Step 10: Set this’s name to name.
        *stored_name = name;
        Ok(())
    }

    /// <https://www.w3.org/TR/IndexedDB/#dom-idbindex-objectstore>
    fn ObjectStore(&self) -> DomRoot<IDBObjectStore> {
        self.object_store.as_rooted()
    }

    /// <https://www.w3.org/TR/IndexedDB/#dom-idbindex-multientry>
    fn MultiEntry(&self) -> bool {
        self.multi_entry
    }

    /// <https://www.w3.org/TR/IndexedDB/#dom-idbindex-unique>
    fn Unique(&self) -> bool {
        self.unique
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-get>
    fn Get(&self, cx: &mut JSContext, query: HandleValue) -> Fallible<DomRoot<IDBRequest>> {
        self.check_usable()?;
        let range = convert_value_to_key_range(cx, query, Some(true))?;
        self.query(cx, range, IndexQueryKind::Get)
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-getkey>
    fn GetKey(&self, cx: &mut JSContext, query: HandleValue) -> Fallible<DomRoot<IDBRequest>> {
        self.check_usable()?;
        let range = convert_value_to_key_range(cx, query, Some(true))?;
        self.query(cx, range, IndexQueryKind::GetKey)
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-getall>
    fn GetAll(
        &self,
        cx: &mut JSContext,
        query: HandleValue,
        count: Option<u32>,
    ) -> Fallible<DomRoot<IDBRequest>> {
        self.check_usable()?;
        let range = convert_value_to_key_range(cx, query, None)?;
        self.query(cx, range, IndexQueryKind::GetAll(count))
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-getallkeys>
    fn GetAllKeys(
        &self,
        cx: &mut JSContext,
        query: HandleValue,
        count: Option<u32>,
    ) -> Fallible<DomRoot<IDBRequest>> {
        self.check_usable()?;
        let range = convert_value_to_key_range(cx, query, None)?;
        self.query(cx, range, IndexQueryKind::GetAllKeys(count))
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-count>
    fn Count(&self, cx: &mut JSContext, query: HandleValue) -> Fallible<DomRoot<IDBRequest>> {
        self.check_usable()?;
        let range = convert_value_to_key_range(cx, query, None)?;
        self.query(cx, range, IndexQueryKind::Count)
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-opencursor>
    fn OpenCursor(
        &self,
        cx: &mut JSContext,
        query: HandleValue,
        direction: IDBCursorDirection,
    ) -> Fallible<DomRoot<IDBRequest>> {
        self.open_cursor(cx, query, direction, false)
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-openkeycursor>
    fn OpenKeyCursor(
        &self,
        cx: &mut JSContext,
        query: HandleValue,
        direction: IDBCursorDirection,
    ) -> Fallible<DomRoot<IDBRequest>> {
        self.open_cursor(cx, query, direction, true)
    }

    /// <https://www.w3.org/TR/IndexedDB/#dom-idbindex-keypath>
    fn KeyPath(&self, cx: &mut JSContext, retval: MutableHandleValue) {
        match &self.key_path {
            KeyPath::String(string) => {
                string.to_jsval(cx, retval);
            },
            KeyPath::StringSequence(sequence) => {
                sequence.to_jsval(cx, retval);
            },
        }
    }
}
