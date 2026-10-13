/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use script_bindings::root::DomRoot;

use crate::dom::html::htmlheadelement::HTMLHeadElement;
use crate::dom::node::NodeTraits;
use crate::dom::window::Window;
use crate::realms::enter_auto_realm;

/// The file name the engine gives a user script that has no source file (Ferrite's
/// compatibility scripts). A page script's name is always a URL, so the name tells the
/// engine's own code apart from the page's.
pub(crate) const FERRITE_USER_SCRIPT: &str = "ferrite-user-script";

/// Whether the innermost running script is an embedder user script rather than a page's.
pub(crate) fn caller_is_user_script(cx: &mut js::context::JSContext) -> bool {
    js::rust::describe_scripted_caller(cx)
        .map(|caller| caller.filename == FERRITE_USER_SCRIPT)
        .unwrap_or(false)
}

pub(crate) fn load_script(head: &HTMLHeadElement) {
    let doc = head.owner_document();
    let userscripts = doc.window().userscripts().to_owned();
    if userscripts.is_empty() {
        return;
    }
    let win = DomRoot::from_ref(doc.window());
    doc.add_delayed_task(task!(UserScriptExecute: |cx, win: DomRoot<Window>| {
        let global_scope = win.as_global_scope();
        let mut realm = enter_auto_realm(cx, global_scope);
        let cx = &mut realm.current_realm();

        for user_script in userscripts {
            _ = global_scope.evaluate_user_script_on_global(
                cx,
                user_script.script().into(),
                &user_script.source_file().map(|path| path.to_string_lossy().to_string()).unwrap_or_else(|| FERRITE_USER_SCRIPT.to_owned()),
            );
        }
    }));
}
