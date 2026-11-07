//! Stand-in for `libjbig-0.dll` (JBIG-KIT, GPL) so the Tesseract bundle Tessera downloads
//! contains no GPL code. libtiff links these ten names at load time but only calls them for
//! JBIG-compressed TIFF; here every call reports an error, so such a TIFF is refused and
//! everything else (PNG, JPEG, ...) is unaffected. Written from the names alone, MIT licensed.

use std::ffi::{c_char, c_int, c_ulong, c_void};

/// Any value other than 0 (ok) and 1 (interrupted) is a failure to the caller.
const ERR: c_int = 0x10;

#[unsafe(no_mangle)]
pub extern "C" fn jbg_dec_init(_s: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn jbg_dec_in(_s: *mut c_void, _d: *const u8, _n: usize, _p: *mut usize) -> c_int {
    ERR
}

#[unsafe(no_mangle)]
pub extern "C" fn jbg_dec_getsize(_s: *const c_void) -> c_ulong {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn jbg_dec_getimage(_s: *const c_void, _plane: c_int) -> *mut u8 {
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn jbg_dec_free(_s: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn jbg_enc_init(
    _s: *mut c_void,
    _x: c_ulong,
    _y: c_ulong,
    _planes: c_int,
    _p: *mut *mut u8,
    _out: *mut c_void,
    _file: *mut c_void,
) {
}

#[unsafe(no_mangle)]
pub extern "C" fn jbg_enc_out(_s: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn jbg_enc_free(_s: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn jbg_newlen(_d: *mut u8, _n: usize) -> c_int {
    ERR
}

#[unsafe(no_mangle)]
pub extern "C" fn jbg_strerror(_code: c_int) -> *const c_char {
    c"JBIG is not supported in this build".as_ptr()
}
