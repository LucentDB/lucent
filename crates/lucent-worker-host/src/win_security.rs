//! User-SID-only named-pipe security. tokio's ServerOptions has no DACL
//! setter (`create()` passes NULL lpSecurityAttributes, which yields the
//! system default pipe DACL: full control for LocalSystem/Administrators/
//! creator-owner, READ for Everyone + Anonymous). We therefore build a
//! SECURITY_ATTRIBUTES whose DACL is a single ACE granting GENERIC_ALL to
//! the current user's SID and pass it through
//! `ServerOptions::create_with_security_attributes_raw`.

use std::io;

use windows_sys::core::PWSTR;
use windows_sys::Win32::Foundation::{LocalFree, ERROR_INSUFFICIENT_BUFFER, HANDLE, HLOCAL};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

pub fn bind_pipe(pipe_name: &str) -> io::Result<crate::ipc::IpcListener> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let attrs = user_only_security_attributes()?;
    let server = unsafe {
        ServerOptions::new()
            .first_pipe_instance(true)
            .create_with_security_attributes_raw(
                pipe_name,
                &attrs as *const SECURITY_ATTRIBUTES as *mut std::ffi::c_void,
            )
    }?;
    Ok(crate::ipc::IpcListener::Pipe(server))
}

fn last_io_err(_what: &str) -> io::Error {
    // The most recent failed Win32 call set the thread's last-error code;
    // surface that (with the operation name for context) to the caller.
    io::Error::last_os_error()
}

fn user_sid_string() -> io::Result<String> {
    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_io_err("OpenProcessToken"));
    }
    // First call sizes the buffer (fails with ERROR_INSUFFICIENT_BUFFER).
    let mut len: u32 = 0;
    unsafe {
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
        return Err(err);
    }
    let mut buf = vec![0u8; len as usize];
    if unsafe { GetTokenInformation(token, TokenUser, buf.as_mut_ptr() as *mut _, len, &mut len) }
        == 0
    {
        return Err(last_io_err("GetTokenInformation"));
    }
    // read_unaligned: GetTokenInformation fills a plain byte buffer whose
    // alignment is not guaranteed to match TOKEN_USER — creating a reference
    // to an insufficiently-aligned value is UB on the security-critical DACL
    // path.
    let token_user = unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const TOKEN_USER) };
    let mut sid_str: PWSTR = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_str) } == 0 {
        return Err(last_io_err("ConvertSidToStringSidW"));
    }
    let wide = unsafe { std::slice::from_raw_parts(sid_str, wcslen(sid_str)) };
    let sid = String::from_utf16_lossy(wide);
    unsafe { LocalFree(sid_str as HLOCAL) };
    Ok(sid)
}

fn wcslen(mut p: *const u16) -> usize {
    let mut n = 0;
    unsafe {
        while *p != 0 {
            n += 1;
            p = p.add(1);
        }
    }
    n
}

fn encode_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// SECURITY_ATTRIBUTES with a protected DACL granting GENERIC_ALL to the
/// current user's SID only. The descriptor is heap-allocated by the SDDL
/// converter; it is owned by the OS pipe object after creation, so it is
/// deliberately leaked (the pipe's lifetime is the process lifetime).
pub fn user_only_security_attributes() -> io::Result<SECURITY_ATTRIBUTES> {
    let sid = user_sid_string()?;
    let sddl = format!("D:P(A;;GA;;;{sid})");
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // Bind the wide buffer so it outlives the FFI call (a temporary would
    // be dropped and leave a dangling pointer).
    let sddl_wide = encode_wide(&sddl);
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl_wide.as_ptr(),
            1,
            &mut sd,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(last_io_err(
            "ConvertStringSecurityDescriptorToSecurityDescriptorW",
        ));
    }
    Ok(SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd as *mut _,
        bInheritHandle: 0,
    })
}
