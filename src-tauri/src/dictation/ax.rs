//! The Accessibility (AX) surface both halves of injection are built on:
//! reading another app's focused element to decide whether there is anywhere
//! to put the text (`paste_target`), and writing the text straight into it
//! (`inject`).
//!
//! Everything `unsafe` about talking to other apps' UI lives here, and no raw
//! pointer escapes: [`Element`] owns its CoreFoundation reference and releases
//! it on drop, so callers deal only in `Option<String>`, `bool` and
//! [`CfRange`]. AX hands back +1 references from every `Copy`/`Create` call,
//! which is the one ownership rule this module exists to get right in a single
//! place.
//!
//! Every read here answers about whichever app the pid names, *live*. An app
//! that is not frontmost generally describes no focused element at all, so
//! callers must activate their target first - see `inject::inject_text`.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2_foundation::NSString;

type CFTypeRef = *const c_void;
type AXUIElementRef = CFTypeRef;
type AXError = i32;
type CFIndex = isize;

const AX_SUCCESS: AXError = 0;

/// `kAXValueTypeCFRange`, the `AXValueRef` payload behind
/// `AXSelectedTextRange`.
const AX_VALUE_CF_RANGE_TYPE: u32 = 4;

/// The selection in the focused element. Setting it replaces the selected text
/// - or inserts at the caret, when nothing is selected.
pub const SELECTED_TEXT: &str = "AXSelectedText";

/// Where that selection sits, as a character range. Read before and after an
/// insertion to tell a real one from a call an app accepted and ignored.
pub const SELECTED_TEXT_RANGE: &str = "AXSelectedTextRange";

/// A character range in a text element, matching CoreFoundation's `CFRange`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CfRange {
    pub location: CFIndex,
    pub length: CFIndex,
}

// AX lives in ApplicationServices (HIServices). Linked explicitly rather than
// relying on AppKit to drag it in.
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: libc::pid_t) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: *const NSString,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementIsAttributeSettable(
        element: AXUIElementRef,
        attribute: *const NSString,
        settable: *mut u8,
    ) -> AXError;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: *const NSString,
        value: CFTypeRef,
    ) -> AXError;
    fn AXValueGetValue(value: CFTypeRef, value_type: u32, out: *mut c_void) -> bool;
    fn CFRelease(cf: CFTypeRef);
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn AXIsProcessTrusted() -> bool;
}

/// Whether this process may read other apps' Accessibility trees at all.
///
/// Without it every attribute read below returns nothing, so every app looks
/// like it "named no focused element" and the paste target is undetectable -
/// which silently degrades `paste_target` into "always paste, never warn".
/// That is not a hypothetical: on 2026-08-20 an ad-hoc-signed reinstall
/// dropped the grant, and because nothing checked or reported it, several
/// rounds of debugging went into the paste-target rules while the real answer
/// was that Scribe could not see anything at all.
pub fn is_trusted() -> bool {
    // SAFETY: a plain predicate with no arguments and no ownership transfer.
    unsafe { AXIsProcessTrusted() }
}

/// Any CoreFoundation value owned by us, released on drop. AX hands back +1
/// references from every `Copy`/`Create` call, and this is the only place that
/// obligation is discharged.
struct OwnedCfType(CFTypeRef);

impl Drop for OwnedCfType {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: non-null and owned - every construction site is a
            // CoreFoundation `Copy`/`Create` call, which returns +1.
            unsafe { CFRelease(self.0) };
        }
    }
}

/// One AX element - an application, a window, or a control inside one.
pub struct Element(OwnedCfType);

impl Element {
    /// The application element for `pid`. `None` only if AX refuses to make
    /// one at all; a dead or unresponsive process still yields an element,
    /// which simply answers errors to everything asked of it.
    pub fn for_app(pid: libc::pid_t) -> Option<Self> {
        // SAFETY: creating an application element is safe for any pid.
        let element = unsafe { AXUIElementCreateApplication(pid) };
        (!element.is_null()).then(|| Self(OwnedCfType(element)))
    }

    /// The element that currently holds keyboard focus inside this app, if the
    /// app names one. Plenty of apps that accept text name nothing here (see
    /// `paste_target`'s module docs), so `None` is not evidence of anything.
    pub fn focused(&self) -> Option<Self> {
        self.attribute_element("AXFocusedUIElement")
    }

    /// Reads an element-valued attribute, taking ownership of the result.
    pub fn attribute_element(&self, attribute: &str) -> Option<Self> {
        self.copy_attribute(attribute).map(Self)
    }

    /// Reads a CFString-valued attribute as a Rust string, via the toll-free
    /// bridge to `NSString`. The type is checked rather than assumed: AX
    /// returns whatever the app put there.
    pub fn attribute_string(&self, attribute: &str) -> Option<String> {
        let value = self.copy_attribute(attribute)?;
        // SAFETY: CFStringGetTypeID/CFGetTypeID are pure reads of live CF values.
        if unsafe { CFGetTypeID(value.0) } != unsafe { CFStringGetTypeID() } {
            return None;
        }
        // SAFETY: confirmed a CFString above, which is toll-free bridged to
        // NSString; the borrow ends before `value` is released.
        let string: &NSString = unsafe { &*(value.0 as *const NSString) };
        Some(string.to_string())
    }

    /// Reads a range-valued attribute (`AXValueRef` wrapping a `CFRange`).
    /// `None` if the attribute is missing or holds some other type.
    pub fn attribute_range(&self, attribute: &str) -> Option<CfRange> {
        let value = self.copy_attribute(attribute)?;
        let mut range = CfRange {
            location: 0,
            length: 0,
        };
        // SAFETY: `value` is a live CF value; `AXValueGetValue` checks the
        // payload type itself and returns false rather than writing `range`
        // when it does not match.
        let ok = unsafe {
            AXValueGetValue(
                value.0,
                AX_VALUE_CF_RANGE_TYPE,
                &mut range as *mut CfRange as *mut c_void,
            )
        };
        ok.then_some(range)
    }

    /// Whether the attribute can be written. The documented capability probe:
    /// an app that will not accept a write says so here, before anything is
    /// attempted.
    pub fn attribute_is_settable(&self, attribute: &str) -> bool {
        let name = NSString::from_str(attribute);
        let mut settable: u8 = 0;
        // SAFETY: `self.0` is a live AX element, `name` outlives the call, and
        // `settable` is only read on success.
        let error = unsafe {
            AXUIElementIsAttributeSettable(
                self.raw(),
                Retained::as_ptr(&name),
                &mut settable as *mut _,
            )
        };
        error == AX_SUCCESS && settable != 0
    }

    /// Writes a string attribute. `Err` carries the raw `AXError` for the log:
    /// apps decline in several distinguishable ways, and which one they chose
    /// is the only clue available when a write does not take.
    pub fn set_attribute_string(&self, attribute: &str, value: &str) -> Result<(), AXError> {
        let name = NSString::from_str(attribute);
        let value = NSString::from_str(value);
        // SAFETY: both strings outlive the call, and NSString is toll-free
        // bridged to the CFStringRef the attribute expects.
        let error = unsafe {
            AXUIElementSetAttributeValue(
                self.raw(),
                Retained::as_ptr(&name),
                Retained::as_ptr(&value) as CFTypeRef,
            )
        };
        if error == AX_SUCCESS {
            Ok(())
        } else {
            Err(error)
        }
    }

    /// Reads one attribute off this element, taking ownership of the result.
    /// `None` for any error, including "no value".
    fn copy_attribute(&self, attribute: &str) -> Option<OwnedCfType> {
        let name = NSString::from_str(attribute);
        let mut value: CFTypeRef = std::ptr::null();
        // SAFETY: `self.0` is a live AX element, `name` outlives the call, and
        // `value` is only read when the call reports success.
        let error = unsafe {
            AXUIElementCopyAttributeValue(self.raw(), Retained::as_ptr(&name), &mut value as *mut _)
        };
        if error != AX_SUCCESS || value.is_null() {
            return None;
        }
        Some(OwnedCfType(value))
    }

    /// The borrowed raw element, for the FFI calls above. Never escapes.
    fn raw(&self) -> AXUIElementRef {
        (self.0).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_process_answers_attribute_reads() {
        // Exercises the whole FFI path (element creation, attribute copy, role
        // read, settability query, release) against a live process, so a
        // mistake in the ownership handling shows up here rather than in
        // dictation. The answers themselves are not asserted - this process is
        // not frontmost during tests - only that asking is survivable.
        let app = Element::for_app(std::process::id() as libc::pid_t)
            .expect("an application element exists for a live pid");
        let _ = app.attribute_string("AXRole");
        let _ = app.attribute_is_settable("AXValue");
        let _ = app.attribute_range(SELECTED_TEXT_RANGE);
        let _ = app.focused();
    }

    #[test]
    fn a_missing_attribute_reads_as_none() {
        let app = Element::for_app(std::process::id() as libc::pid_t).expect("element");
        assert!(app.attribute_string("AXDefinitelyNotAnAttribute").is_none());
        assert!(app.attribute_range("AXDefinitelyNotAnAttribute").is_none());
        assert!(!app.attribute_is_settable("AXDefinitelyNotAnAttribute"));
    }

    #[test]
    fn writing_an_unsupported_attribute_reports_the_ax_error() {
        // The fallback in `inject` hangs off this being an `Err`, not a panic
        // or a silent success.
        let app = Element::for_app(std::process::id() as libc::pid_t).expect("element");
        assert!(app.set_attribute_string(SELECTED_TEXT, "nope").is_err());
    }
}
