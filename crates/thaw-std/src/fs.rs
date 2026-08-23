use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;

fn string_from_ptr(value: *const c_char) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}

#[no_mangle]
pub extern "C" fn existsSync(path: *const c_char) -> u8 {
    Path::new(&string_from_ptr(path)).exists() as u8
}

#[no_mangle]
pub extern "C" fn readFileSync(path: *const c_char, encoding: *const c_char) -> *const c_char {
    let path = string_from_ptr(path);
    let encoding = string_from_ptr(encoding);
    let value = if encoding == "utf8" || encoding == "utf-8" {
        std::fs::read_to_string(path).unwrap_or_default()
    } else {
        String::new()
    };
    CString::new(value).unwrap_or_default().into_raw()
}

#[no_mangle]
pub extern "C" fn writeFileSync(path: *const c_char, data: *const c_char) -> u8 {
    let path = string_from_ptr(path);
    let data = string_from_ptr(data);
    std::fs::write(path, data).is_ok() as u8
}

#[no_mangle]
pub extern "C" fn mkdirSync(path: *const c_char) -> u8 {
    let path = string_from_ptr(path);
    (Path::new(&path).is_dir() || std::fs::create_dir_all(path).is_ok()) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_reads_and_checks_utf8_files() {
        let dir = std::env::temp_dir().join(format!("thaw-std-fs-{}", std::process::id()));
        let dir_c = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        assert_eq!(mkdirSync(dir_c.as_ptr()), 1);
        assert_eq!(existsSync(dir_c.as_ptr()), 1);
        let file = dir.join("message.txt");
        let file_c = CString::new(file.to_string_lossy().as_bytes()).unwrap();
        let data = CString::new("hello thaw").unwrap();
        assert_eq!(writeFileSync(file_c.as_ptr(), data.as_ptr()), 1);
        let encoding = CString::new("utf8").unwrap();
        let result = readFileSync(file_c.as_ptr(), encoding.as_ptr());
        assert_eq!(
            unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
            "hello thaw"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
