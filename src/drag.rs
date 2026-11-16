//! Native CF_HDROP file dragging. The receiving app decides what to do on drop.
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
};
use windows::{
    core::{BOOL, HRESULT, PCWSTR},
    Win32::{
        Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK},
        System::{
            Com::IDataObject,
            Ole::*,
            SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS},
        },
        UI::Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName},
    },
};

#[derive(Clone, Default)]
pub struct Control {
    cancel: Arc<AtomicBool>,
    thread: Arc<AtomicU32>,
}
impl Control {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        let id = self.thread.load(Ordering::SeqCst);
        if id != 0 {
            // Wake OLE's mouse loop so a stationary drag can also be cancelled on quit.
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                    id,
                    windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE,
                    0,
                    0,
                );
            }
        }
    }
}
struct ThreadRegistration(Control);
impl Drop for ThreadRegistration {
    fn drop(&mut self) {
        self.0.thread.store(0, Ordering::SeqCst);
    }
}

#[windows::core::implement(IDropSource)]
struct Source {
    cancel: Arc<AtomicBool>,
}
impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() || self.cancel.load(Ordering::SeqCst) {
            DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }
    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            OleUninitialize();
        }
    }
}

fn data_object(path: &Path) -> windows::core::Result<IDataObject> {
    use std::os::windows::ffi::OsStrExt;
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let item: IShellItem = unsafe { SHCreateItemFromParsingName(PCWSTR(path.as_ptr()), None) }?;
    unsafe { item.BindToHandler(None, &BHID_DataObject) }
}

pub fn file(path: &Path, control: Control) -> Result<(), String> {
    if !path.is_file() {
        return Err("Der Clip existiert nicht mehr.".into());
    }
    unsafe { OleInitialize(None) }.map_err(|e| e.to_string())?;
    let _apartment = Apartment;
    control.thread.store(
        unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() },
        Ordering::SeqCst,
    );
    let _registration = ThreadRegistration(control.clone());
    if control.cancel.load(Ordering::SeqCst) {
        return Ok(());
    }
    let data = data_object(path).map_err(|e| format!("Datei konnte nicht gezogen werden: {e}"))?;
    let source: IDropSource = Source {
        cancel: control.cancel,
    }
    .into();
    let mut effect = DROPEFFECT_NONE;
    unsafe { DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect) }
        .ok()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_drag_contains_the_real_unicode_file_path() {
        use windows::Win32::{
            System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL},
            UI::Shell::{DragQueryFileW, HDROP},
        };
        unsafe { OleInitialize(None) }.unwrap();
        let _apartment = Apartment;
        let path = std::env::temp_dir().join(format!("stillclip-ä-{}-🎮.mp4", std::process::id()));
        std::fs::write(&path, b"synthetic drag fixture").unwrap();
        let data = data_object(&path).unwrap();
        let format = FORMATETC {
            cfFormat: 15,
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
            ..Default::default()
        };
        let mut medium = unsafe { data.GetData(&format) }.unwrap();
        let drop = HDROP(unsafe { medium.u.hGlobal.0 });
        let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
        let length = unsafe { DragQueryFileW(drop, 0, None) };
        let mut actual = vec![0_u16; length as usize + 1];
        unsafe {
            DragQueryFileW(drop, 0, Some(&mut actual));
            ReleaseStgMedium(&mut medium);
        }
        let actual =
            std::path::PathBuf::from(String::from_utf16(&actual[..length as usize]).unwrap());
        // The shell expands 8.3 names (e.g. RUNNER~1) to their long spelling.
        let actual_file = actual.canonicalize().unwrap();
        let expected_file = path.canonicalize().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(count, 1);
        assert_eq!(actual.file_name(), path.file_name());
        assert_eq!(actual_file, expected_file);
    }
}
