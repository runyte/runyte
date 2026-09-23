# SPDX-License-Identifier: MPL-2.0
"""Private Windows storage and named-pipe transport for the context bridge."""

import ctypes
from ctypes import wintypes
import math
import msvcrt
import os
from pathlib import Path, PureWindowsPath
import re
import secrets
import subprocess
import threading
import time


kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
advapi32 = ctypes.WinDLL("advapi32", use_last_error=True)
shell32 = ctypes.WinDLL("shell32", use_last_error=True)
ole32 = ctypes.WinDLL("ole32", use_last_error=True)
ntdll = ctypes.WinDLL("ntdll", use_last_error=True)

INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value
GENERIC_READ = 0x80000000
GENERIC_WRITE = 0x40000000
FILE_GENERIC_READ = 0x00120089
SYNCHRONIZE = 0x00100000
FILE_SHARE_READ = 1
FILE_SHARE_WRITE = 2
FILE_SHARE_DELETE = 4
OPEN_EXISTING = 3
FILE_FLAG_BACKUP_SEMANTICS = 0x02000000
FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000
FILE_FLAG_OVERLAPPED = 0x40000000
SECURITY_SQOS_PRESENT = 0x00100000
SECURITY_IDENTIFICATION = 0x00010000
FILE_ATTRIBUTE_DIRECTORY = 0x10
FILE_ATTRIBUTE_REPARSE_POINT = 0x400
FILE_TYPE_PIPE = 3
FILE_PERSISTENT_ACLS = 0x00000008
FILE_REMOTE_DEVICE = 0x10
FILE_DIRECTORY_FILE = 0x00000001
FILE_NON_DIRECTORY_FILE = 0x00000040
FILE_SYNCHRONOUS_IO_NONALERT = 0x00000020
FILE_OPEN_REPARSE_POINT = 0x00200000
FILE_OPEN = 1
OBJ_CASE_INSENSITIVE = 0x40
OBJ_DONT_REPARSE = 0x1000
ERROR_FILE_NOT_FOUND = 2
ERROR_HANDLE_EOF = 38
ERROR_NETNAME_DELETED = 64
ERROR_BROKEN_PIPE = 109
ERROR_NO_DATA = 232
ERROR_PIPE_NOT_CONNECTED = 233
ERROR_PIPE_BUSY = 231
ERROR_IO_PENDING = 997
ERROR_OPERATION_ABORTED = 995
WAIT_OBJECT_0 = 0
WAIT_TIMEOUT = 258
WAIT_FAILED = 0xFFFFFFFF
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
TOKEN_QUERY = 0x0008
TokenUser = 1
SE_FILE_OBJECT = 1
OWNER_SECURITY_INFORMATION = 0x00000001
DACL_SECURITY_INFORMATION = 0x00000004
SE_DACL_PRESENT = 0x0004
SE_DACL_PROTECTED = 0x1000
GENERIC_ALL = 0x10000000
FILE_ALL_ACCESS = 0x001F01FF
MAX_U64 = (1 << 64) - 1
FILE_FLAG_FIRST_PIPE_INSTANCE = 0x00080000
PIPE_ACCESS_INBOUND = 0x00000001
PIPE_REJECT_REMOTE_CLIENTS = 0x00000008
CREATE_NO_WINDOW = 0x08000000
DUPLICATE_SAME_ACCESS = 0x00000002


class NotSubmitted(TimeoutError):
    """The absolute deadline or cancellation won before native submission."""


class GUID(ctypes.Structure):
    _fields_ = [("Data1", wintypes.DWORD), ("Data2", wintypes.WORD),
                ("Data3", wintypes.WORD), ("Data4", ctypes.c_ubyte * 8)]


class FILETIME(ctypes.Structure):
    _fields_ = [("dwLowDateTime", wintypes.DWORD),
                ("dwHighDateTime", wintypes.DWORD)]


class BY_HANDLE_FILE_INFORMATION(ctypes.Structure):
    _fields_ = [("dwFileAttributes", wintypes.DWORD),
                ("ftCreationTime", FILETIME), ("ftLastAccessTime", FILETIME),
                ("ftLastWriteTime", FILETIME), ("dwVolumeSerialNumber", wintypes.DWORD),
                ("nFileSizeHigh", wintypes.DWORD), ("nFileSizeLow", wintypes.DWORD),
                ("nNumberOfLinks", wintypes.DWORD), ("nFileIndexHigh", wintypes.DWORD),
                ("nFileIndexLow", wintypes.DWORD)]


class UNICODE_STRING(ctypes.Structure):
    _fields_ = [("Length", wintypes.USHORT), ("MaximumLength", wintypes.USHORT),
                ("Buffer", wintypes.LPWSTR)]


class OBJECT_ATTRIBUTES(ctypes.Structure):
    _fields_ = [("Length", wintypes.ULONG), ("RootDirectory", wintypes.HANDLE),
                ("ObjectName", ctypes.POINTER(UNICODE_STRING)),
                ("Attributes", wintypes.ULONG), ("SecurityDescriptor", wintypes.LPVOID),
                ("SecurityQualityOfService", wintypes.LPVOID)]


class IO_STATUS_BLOCK(ctypes.Structure):
    _fields_ = [("Status", ctypes.c_ssize_t), ("Information", ctypes.c_size_t)]


class OVERLAPPED(ctypes.Structure):
    _fields_ = [("Internal", ctypes.c_size_t), ("InternalHigh", ctypes.c_size_t),
                ("Offset", wintypes.DWORD), ("OffsetHigh", wintypes.DWORD),
                ("hEvent", wintypes.HANDLE)]


class ACL(ctypes.Structure):
    _fields_ = [("AclRevision", ctypes.c_ubyte), ("Sbz1", ctypes.c_ubyte),
                ("AclSize", wintypes.WORD), ("AceCount", wintypes.WORD),
                ("Sbz2", wintypes.WORD)]


class ACE_HEADER(ctypes.Structure):
    _fields_ = [("AceType", ctypes.c_ubyte), ("AceFlags", ctypes.c_ubyte),
                ("AceSize", wintypes.WORD)]


class ACCESS_ALLOWED_ACE(ctypes.Structure):
    _fields_ = [("Header", ACE_HEADER), ("Mask", wintypes.DWORD),
                ("SidStart", wintypes.DWORD)]


class SID_AND_ATTRIBUTES(ctypes.Structure):
    _fields_ = [("Sid", wintypes.LPVOID), ("Attributes", wintypes.DWORD)]


class TOKEN_USER(ctypes.Structure):
    _fields_ = [("User", SID_AND_ATTRIBUTES)]


class SECURITY_ATTRIBUTES(ctypes.Structure):
    _fields_ = [("nLength", wintypes.DWORD), ("lpSecurityDescriptor", wintypes.LPVOID),
                ("bInheritHandle", wintypes.BOOL)]


kernel32.CreateFileW.restype = wintypes.HANDLE
kernel32.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                 wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD,
                                 wintypes.HANDLE]
kernel32.CreateEventW.restype = wintypes.HANDLE
kernel32.CreateEventW.argtypes = [wintypes.LPVOID, wintypes.BOOL, wintypes.BOOL,
                                  wintypes.LPCWSTR]
kernel32.OpenProcess.restype = wintypes.HANDLE
kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
kernel32.GetCurrentProcess.restype = wintypes.HANDLE
kernel32.CreateNamedPipeW.restype = wintypes.HANDLE
kernel32.CreateNamedPipeW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                      wintypes.DWORD, wintypes.DWORD, wintypes.DWORD,
                                      wintypes.DWORD, ctypes.POINTER(SECURITY_ATTRIBUTES)]
kernel32.LocalFree.restype = wintypes.LPVOID
kernel32.LocalFree.argtypes = [wintypes.LPVOID]
kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
kernel32.DuplicateHandle.argtypes = [wintypes.HANDLE, wintypes.HANDLE, wintypes.HANDLE,
                                     ctypes.POINTER(wintypes.HANDLE), wintypes.DWORD,
                                     wintypes.BOOL, wintypes.DWORD]
kernel32.GetFileType.argtypes = [wintypes.HANDLE]
kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
kernel32.WaitForSingleObject.restype = wintypes.DWORD
kernel32.WaitNamedPipeW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD]
kernel32.PeekNamedPipe.argtypes = [wintypes.HANDLE, wintypes.LPVOID, wintypes.DWORD,
                                   ctypes.POINTER(wintypes.DWORD),
                                   ctypes.POINTER(wintypes.DWORD),
                                   ctypes.POINTER(wintypes.DWORD)]
kernel32.GetFileInformationByHandle.argtypes = [wintypes.HANDLE,
                                                ctypes.POINTER(BY_HANDLE_FILE_INFORMATION)]
kernel32.GetFileSizeEx.argtypes = [wintypes.HANDLE, ctypes.POINTER(ctypes.c_longlong)]
kernel32.GetNamedPipeServerProcessId.argtypes = [wintypes.HANDLE,
                                                 ctypes.POINTER(wintypes.ULONG)]
kernel32.GetProcessTimes.argtypes = [wintypes.HANDLE, ctypes.POINTER(FILETIME),
                                     ctypes.POINTER(FILETIME), ctypes.POINTER(FILETIME),
                                     ctypes.POINTER(FILETIME)]
kernel32.ReadFile.argtypes = [wintypes.HANDLE, wintypes.LPVOID, wintypes.DWORD,
                              ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID]
kernel32.WriteFile.argtypes = kernel32.ReadFile.argtypes
kernel32.CancelIoEx.argtypes = [wintypes.HANDLE, wintypes.LPVOID]
kernel32.GetOverlappedResult.argtypes = [wintypes.HANDLE, ctypes.POINTER(OVERLAPPED),
                                         ctypes.POINTER(wintypes.DWORD), wintypes.BOOL]
advapi32.OpenProcessToken.argtypes = [wintypes.HANDLE, wintypes.DWORD,
                                      ctypes.POINTER(wintypes.HANDLE)]
advapi32.GetTokenInformation.argtypes = [wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID,
                                         wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
advapi32.GetSecurityInfo.argtypes = [wintypes.HANDLE, ctypes.c_int, wintypes.DWORD,
                                     ctypes.POINTER(wintypes.LPVOID),
                                     ctypes.POINTER(wintypes.LPVOID),
                                     ctypes.POINTER(ctypes.POINTER(ACL)),
                                     ctypes.POINTER(ctypes.POINTER(ACL)),
                                     ctypes.POINTER(wintypes.LPVOID)]
advapi32.EqualSid.argtypes = [wintypes.LPVOID, wintypes.LPVOID]
advapi32.IsValidSid.argtypes = [wintypes.LPVOID]
advapi32.IsValidAcl.argtypes = [ctypes.POINTER(ACL)]
advapi32.GetLengthSid.argtypes = [wintypes.LPVOID]
advapi32.IsWellKnownSid.argtypes = [wintypes.LPVOID, ctypes.c_int]
advapi32.GetAce.argtypes = [ctypes.POINTER(ACL), wintypes.DWORD,
                            ctypes.POINTER(wintypes.LPVOID)]
advapi32.GetSecurityDescriptorControl.argtypes = [wintypes.LPVOID,
                                                  ctypes.POINTER(wintypes.WORD),
                                                  ctypes.POINTER(wintypes.DWORD)]
shell32.SHGetKnownFolderPath.argtypes = [ctypes.POINTER(GUID), wintypes.DWORD,
                                        wintypes.HANDLE, ctypes.POINTER(wintypes.LPWSTR)]
ole32.CoTaskMemFree.argtypes = [wintypes.LPVOID]
ole32.CoTaskMemFree.restype = None
kernel32.GetVolumeInformationByHandleW.argtypes = [wintypes.HANDLE, wintypes.LPWSTR,
                                                   wintypes.DWORD,
                                                   ctypes.POINTER(wintypes.DWORD),
                                                   ctypes.POINTER(wintypes.DWORD),
                                                   ctypes.POINTER(wintypes.DWORD),
                                                   wintypes.LPWSTR, wintypes.DWORD]
ntdll.NtCreateFile.argtypes = [ctypes.POINTER(wintypes.HANDLE), wintypes.DWORD,
                               ctypes.POINTER(OBJECT_ATTRIBUTES),
                               ctypes.POINTER(IO_STATUS_BLOCK), wintypes.LPVOID,
                               wintypes.DWORD, wintypes.DWORD, wintypes.DWORD,
                               wintypes.DWORD, wintypes.LPVOID, wintypes.DWORD]
ntdll.NtCreateFile.restype = ctypes.c_long
ntdll.NtQueryVolumeInformationFile.argtypes = [wintypes.HANDLE,
                                               ctypes.POINTER(IO_STATUS_BLOCK),
                                               wintypes.LPVOID, wintypes.DWORD, ctypes.c_int]
ntdll.NtQueryVolumeInformationFile.restype = ctypes.c_long
ntdll.RtlNtStatusToDosError.argtypes = [ctypes.c_long]
ntdll.RtlNtStatusToDosError.restype = wintypes.ULONG


class _Handle:
    def __init__(self, value):
        if value in (None, 0, INVALID_HANDLE_VALUE):
            raise ctypes.WinError(ctypes.get_last_error())
        self.value = value

    def close(self):
        if self.value not in (None, 0, INVALID_HANDLE_VALUE):
            kernel32.CloseHandle(self.value)
            self.value = None

    def __del__(self):
        self.close()


def _failed(message):
    error = ctypes.get_last_error()
    if error:
        return OSError(error, message)
    return OSError(message)


def _filetime(value):
    return (int(value.dwHighDateTime) << 32) | int(value.dwLowDateTime)


def storage_root():
    # FOLDERID_LocalAppData is an OS identity, rather than an environment alias.
    folder = GUID(0xF1B32785, 0x6FBA, 0x4FCF,
                  (ctypes.c_ubyte * 8)(0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91))
    value = wintypes.LPWSTR()
    result = shell32.SHGetKnownFolderPath(ctypes.byref(folder), 0, None,
                                          ctypes.byref(value))
    if result != 0:
        raise OSError(result, "Cannot resolve LocalAppData for context storage")
    try:
        return Path(value.value) / "runyte" / "context"
    finally:
        ole32.CoTaskMemFree(ctypes.cast(value, wintypes.LPVOID))


def _drive_root(path):
    parsed = PureWindowsPath(path)
    drive = parsed.drive
    if re.fullmatch(r"\\\\\?\\[A-Za-z]:", drive):
        drive = drive[-2:]
    if not parsed.is_absolute() or not re.fullmatch(r"[A-Za-z]:", drive):
        raise OSError("Context storage requires an absolute local drive path")
    names = list(parsed.parts[1:])
    if not names or any(name in ("", ".", "..") for name in names):
        raise OSError("Invalid context storage path")
    return f"\\\\?\\{drive}\\", names


def _regular(handle, directory):
    info = BY_HANDLE_FILE_INFORMATION()
    if not kernel32.GetFileInformationByHandle(handle, ctypes.byref(info)):
        raise _failed("Cannot inspect context storage")
    attributes = info.dwFileAttributes
    if (attributes & FILE_ATTRIBUTE_REPARSE_POINT
            or bool(attributes & FILE_ATTRIBUTE_DIRECTORY) != directory
            or (not directory and info.nNumberOfLinks != 1)):
        raise OSError("Context storage requires non-reparse directories and single-link files")


def _ntstatus(status, message):
    if status < 0:
        code = ntdll.RtlNtStatusToDosError(status)
        raise OSError(code, message)


def _relative(parent, name, directory):
    if (not name or name in (".", "..") or len(name) > 255
            or name.endswith((" ", ".")) or any(ord(char) < 32 or char in '<>:"|?*' for char in name)):
        raise OSError("Invalid context storage component")
    stem = name.split(".", 1)[0].rstrip().upper()
    if (stem in {"CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"}
            or (stem.startswith(("COM", "LPT"))
                and stem[3:] in {"1", "2", "3", "4", "5", "6", "7", "8", "9",
                                 "\u00b9", "\u00b2", "\u00b3"})):
        raise OSError("Invalid context storage component")
    encoded = name.encode("utf-16-le", errors="surrogatepass")
    buffer = ctypes.create_unicode_buffer(name)
    string = UNICODE_STRING(len(encoded), len(encoded),
                            ctypes.cast(buffer, wintypes.LPWSTR))
    attributes = OBJECT_ATTRIBUTES(ctypes.sizeof(OBJECT_ATTRIBUTES), parent,
                                   ctypes.pointer(string),
                                   OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE, None, None)
    result = wintypes.HANDLE()
    status = IO_STATUS_BLOCK()
    options = (FILE_DIRECTORY_FILE if directory else FILE_NON_DIRECTORY_FILE)
    options |= FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT
    code = ntdll.NtCreateFile(ctypes.byref(result), FILE_GENERIC_READ,
                              ctypes.byref(attributes), ctypes.byref(status), None, 0,
                              FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                              FILE_OPEN, options, None, 0)
    _ntstatus(code, "Cannot open pinned context storage component")
    handle = _Handle(result.value)
    try:
        _regular(handle.value, directory)
        return handle
    except BaseException:
        handle.close()
        raise


def _local_ntfs(handle):
    status = IO_STATUS_BLOCK()
    device = (wintypes.ULONG * 2)()
    code = ntdll.NtQueryVolumeInformationFile(handle, ctypes.byref(status),
                                               ctypes.byref(device), ctypes.sizeof(device), 4)
    _ntstatus(code, "Cannot inspect context storage volume")
    if device[1] & FILE_REMOTE_DEVICE:
        raise OSError("Context storage requires a local volume")
    flags = wintypes.DWORD()
    filesystem = ctypes.create_unicode_buffer(32)
    if not kernel32.GetVolumeInformationByHandleW(handle, None, 0, None, None,
                                                   ctypes.byref(flags), filesystem, 32):
        raise _failed("Cannot inspect context storage filesystem")
    if filesystem.value.upper() != "NTFS" or not flags.value & FILE_PERSISTENT_ACLS:
        raise OSError("Context storage requires local NTFS with persistent ACLs")


def _token_user(process=None):
    token = wintypes.HANDLE()
    process = process or kernel32.GetCurrentProcess()
    if not advapi32.OpenProcessToken(process, TOKEN_QUERY, ctypes.byref(token)):
        raise _failed("Cannot inspect process identity")
    owner = _Handle(token.value)
    try:
        length = wintypes.DWORD()
        advapi32.GetTokenInformation(owner.value, TokenUser, None, 0, ctypes.byref(length))
        if not ctypes.sizeof(TOKEN_USER) <= length.value <= 4096:
            raise OSError("Invalid process identity")
        buffer = ctypes.create_string_buffer(length.value)
        if not advapi32.GetTokenInformation(owner.value, TokenUser, buffer, length,
                                             ctypes.byref(length)):
            raise _failed("Cannot read process identity")
        sid = ctypes.cast(buffer, ctypes.POINTER(TOKEN_USER)).contents.User.Sid
        return buffer, sid
    finally:
        owner.close()


def _private_acl(handle):
    descriptor = wintypes.LPVOID()
    owner = wintypes.LPVOID()
    acl = ctypes.POINTER(ACL)()
    result = advapi32.GetSecurityInfo(handle, SE_FILE_OBJECT,
                                      OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                                      ctypes.byref(owner), None, ctypes.byref(acl), None,
                                      ctypes.byref(descriptor))
    if result:
        raise OSError(result, "Cannot inspect context storage security")
    try:
        user_buffer, user = _token_user()
        if not owner or not advapi32.IsValidSid(owner) or not advapi32.EqualSid(owner, user):
            raise PermissionError("Context storage is not owned by the current user")
        control = wintypes.WORD()
        revision = wintypes.DWORD()
        if not advapi32.GetSecurityDescriptorControl(descriptor, ctypes.byref(control),
                                                      ctypes.byref(revision)):
            raise _failed("Cannot inspect context storage ACL")
        if (not control.value & SE_DACL_PROTECTED or not control.value & SE_DACL_PRESENT
                or not acl or not advapi32.IsValidAcl(acl) or acl.contents.AceCount != 1):
            raise PermissionError("Context storage requires a protected owner-only ACL")
        entry = wintypes.LPVOID()
        if not advapi32.GetAce(acl, 0, ctypes.byref(entry)):
            raise _failed("Cannot inspect context storage ACL entry")
        allowed = ctypes.cast(entry, ctypes.POINTER(ACCESS_ALLOWED_ACE)).contents
        if (allowed.Header.AceType != 0 or allowed.Header.AceFlags != 0
                or allowed.Header.AceSize < ctypes.sizeof(ACCESS_ALLOWED_ACE)):
            raise PermissionError("Context storage requires a protected owner-only ACL")
        sid_address = int(entry.value) + ACCESS_ALLOWED_ACE.SidStart.offset
        sid = ctypes.c_void_p(sid_address)
        sid_bytes = allowed.Header.AceSize - ACCESS_ALLOWED_ACE.SidStart.offset
        subauthorities = ctypes.c_ubyte.from_address(sid_address + 1).value if sid_bytes >= 8 else 255
        if (sid_bytes != 8 + subauthorities * ctypes.sizeof(wintypes.DWORD)
                or not advapi32.IsValidSid(sid)
                or advapi32.GetLengthSid(sid) != sid_bytes
                or (not advapi32.EqualSid(sid, user)
                    and not advapi32.IsWellKnownSid(sid, 71))
                or not (allowed.Mask & GENERIC_ALL
                        or allowed.Mask & FILE_ALL_ACCESS == FILE_ALL_ACCESS)):
            raise PermissionError("Context storage requires a protected owner-only full-access ACL")
    finally:
        kernel32.LocalFree(descriptor)


def _open_directory(path):
    root, names = _drive_root(path)
    value = kernel32.CreateFileW(root, FILE_GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE |
                                 FILE_SHARE_DELETE, None, OPEN_EXISTING,
                                 FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, None)
    current = _Handle(value)
    try:
        _regular(current.value, True)
        _local_ntfs(current.value)
        for name in names:
            following = _relative(current.value, name, True)
            current.close()
            current = following
        _private_acl(current.value)
        return current
    except BaseException:
        current.close()
        raise


def load_credential(root, name):
    from .client import Failure, decode

    if not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", name):
        raise Failure("invalid_argument", "Invalid bridge identity name")
    filename = "identity.json" if name == "agent" else f"identity-{name}.json"
    directory = None
    identity = None
    try:
        directory = _open_directory(root)
        identity = _relative(directory.value, filename, False)
        _private_acl(identity.value)
        size = ctypes.c_longlong()
        if not kernel32.GetFileSizeEx(identity.value, ctypes.byref(size)):
            raise _failed("Cannot inspect bridge identity")
        if size.value > 65536:
            raise Failure("limit_exceeded", "Identity record exceeds limit")
        raw = bytearray()
        while len(raw) <= 65536:
            chunk = ctypes.create_string_buffer(min(65537 - len(raw), 65536))
            count = wintypes.DWORD()
            if not kernel32.ReadFile(identity.value, chunk, len(chunk), ctypes.byref(count), None):
                raise _failed("Cannot read bridge identity")
            if not count.value:
                break
            raw.extend(chunk.raw[:count.value])
        if len(raw) > 65536:
            raise Failure("limit_exceeded", "Identity record exceeds limit")
        value = decode(bytes(raw))
        if (not isinstance(value, dict) or set(value) != {"name", "credential"}
                or value["name"] != name or not isinstance(value["credential"], str)
                or not re.fullmatch(r"[0-9a-f]{64}", value["credential"])):
            raise Failure("unavailable", "Invalid bridge identity record")
        return value["credential"]
    except Failure:
        raise
    except OSError:
        raise Failure("unavailable", "Pair this identity in Runyte before connecting") from None
    finally:
        if identity:
            identity.close()
        if directory:
            directory.close()


def validate_record(record):
    endpoint = record.get("endpoint")
    pid = record.get("pid")
    creation = record.get("creation_time")
    incarnation = record.get("host_incarnation")
    if (not isinstance(endpoint, str)
            or not re.fullmatch(r"\\\\\.\\pipe\\runyte-context-v1-[0-9a-f]{64}", endpoint)
            or not isinstance(incarnation, str) or not re.fullmatch(r"[0-9a-f]{64}", incarnation)
            or endpoint.rsplit("-", 1)[-1] != incarnation
            or not isinstance(record.get("workspace_id"), str)
            or not re.fullmatch(r"[0-9a-f]{64}", record["workspace_id"])
            or not isinstance(record.get("root"), str) or not record["root"]
            or record.get("mode") not in ("persistent", "standalone")
            or not isinstance(record.get("environment"), str)
            or not re.fullmatch(r"[0-9a-f]{64}", record["environment"])
            or isinstance(pid, bool) or not isinstance(pid, int) or not 1 <= pid <= 0xFFFFFFFF
            or isinstance(creation, bool) or not isinstance(creation, int)
            or not 1 <= creation <= MAX_U64):
        raise OSError("Invalid Windows context registration")


def _process(pid, creation):
    value = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, False, pid)
    process = _Handle(value)
    try:
        created, exited, kernel, user = FILETIME(), FILETIME(), FILETIME(), FILETIME()
        if not kernel32.GetProcessTimes(process.value, ctypes.byref(created), ctypes.byref(exited),
                                        ctypes.byref(kernel), ctypes.byref(user)):
            raise _failed("Cannot inspect context host process")
        if _filetime(created) != creation:
            raise PermissionError("Context host process identity changed")
        if kernel32.WaitForSingleObject(process.value, 0) != WAIT_TIMEOUT:
            raise PermissionError("Context host process has exited")
        current_buffer, current = _token_user()
        peer_buffer, peer = _token_user(process.value)
        try:
            if not advapi32.EqualSid(current, peer):
                raise PermissionError("Context host belongs to a different account")
        finally:
            del current_buffer, peer_buffer
        if kernel32.WaitForSingleObject(process.value, 0) != WAIT_TIMEOUT:
            raise PermissionError("Context host process exited during admission")
        return process
    except BaseException:
        process.close()
        raise


_NATIVE_IO_CAPACITY = 8
_NATIVE_IO_SLOTS = threading.BoundedSemaphore(_NATIVE_IO_CAPACITY)
_NATIVE_IO_STATE = threading.Lock()
_RETAINED_OPERATIONS = []
_NATIVE_IO_DISABLED = False


def _terminal_completion(error):
    return error in {
        ERROR_HANDLE_EOF,
        ERROR_NETNAME_DELETED,
        ERROR_BROKEN_PIPE,
        ERROR_NO_DATA,
        ERROR_PIPE_NOT_CONNECTED,
        ERROR_OPERATION_ABORTED,
    }


def _retain_operation(owner):
    global _NATIVE_IO_DISABLED
    with _NATIVE_IO_STATE:
        _NATIVE_IO_DISABLED = True
        # Every operation owns one of exactly eight admission slots. A retained
        # owner never returns its slot, so this inventory is intrinsically
        # bounded even when all admitted operations fail concurrently.
        if len(_RETAINED_OPERATIONS) >= _NATIVE_IO_CAPACITY:
            raise RuntimeError("native context I/O retention capacity exhausted")
        _RETAINED_OPERATIONS.append(owner)


class _Operation:
    """One interrupt-resistant owner for an overlapped native operation."""

    def __init__(self, handle, write, data, deadline):
        process = kernel32.GetCurrentProcess()
        duplicate = wintypes.HANDLE()
        if not kernel32.DuplicateHandle(process, handle, process, ctypes.byref(duplicate),
                                        0, False, DUPLICATE_SAME_ACCESS):
            raise _failed("Cannot retain context pipe operation")
        self.handle = _Handle(duplicate.value)
        try:
            self.event = _Handle(kernel32.CreateEventW(None, True, False, None))
        except BaseException:
            self.handle.close()
            raise
        self.overlapped = OVERLAPPED(hEvent=self.event.value)
        self.count = wintypes.DWORD()
        self.write = write
        self.buffer = (ctypes.create_string_buffer(data) if write
                       else ctypes.create_string_buffer(data))
        self.length = len(data) if write else data
        self.deadline = deadline
        self.cancel_requested = False
        self.result = None
        self.error = None
        self.settled = False
        self.owns_slot = True
        self.started = threading.Event()
        self.completed = threading.Event()

    def request_cancel(self):
        self.cancel_requested = True
        # Only the owner thread touches its duplicated handle. It checks this
        # sticky request after submission; a later race remains bounded by the
        # original absolute deadline before the owner cancels and drains.

    def _drain(self):
        kernel32.CancelIoEx(self.handle.value, ctypes.byref(self.overlapped))
        if kernel32.GetOverlappedResult(self.handle.value, ctypes.byref(self.overlapped),
                                        ctypes.byref(self.count), True):
            self.settled = True
            return
        error = ctypes.get_last_error()
        if _terminal_completion(error):
            self.settled = True
            return
        raise OSError(error, "Cannot confirm cancellation of native pipe I/O")

    def entry(self):
        self.started.set()
        try:
            self.run()
        finally:
            self.completed.set()
            if self.settled and self.owns_slot:
                self.owns_slot = False
                _NATIVE_IO_SLOTS.release()

    def run(self):
        try:
            if self.cancel_requested:
                self.settled = True
                raise NotSubmitted("Native pipe operation was cancelled before submission")
            if self.deadline <= time.monotonic():
                self.settled = True
                raise NotSubmitted("Native pipe deadline expired before submission")
            if self.write:
                success = kernel32.WriteFile(self.handle.value, self.buffer, self.length,
                                             ctypes.byref(self.count),
                                             ctypes.byref(self.overlapped))
            else:
                success = kernel32.ReadFile(self.handle.value, self.buffer, self.length,
                                            ctypes.byref(self.count),
                                            ctypes.byref(self.overlapped))
            error = 0 if success else ctypes.get_last_error()
            if success:
                self.settled = True
            elif error == ERROR_BROKEN_PIPE and not self.write:
                self.settled = True
                self.result = b""
                return
            elif error != ERROR_IO_PENDING:
                self.settled = True
                raise ctypes.WinError(error)
            if self.cancel_requested and not self.settled:
                kernel32.CancelIoEx(self.handle.value, ctypes.byref(self.overlapped))
            if not self.settled:
                remaining = self.deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError()
                wait = kernel32.WaitForSingleObject(
                    self.event.value,
                    max(1, min(0xFFFFFFFE, math.ceil(remaining * 1000))),
                )
                if wait == WAIT_TIMEOUT:
                    raise TimeoutError()
                if wait == WAIT_FAILED:
                    raise _failed("Cannot wait for native pipe I/O")
                if wait != WAIT_OBJECT_0:
                    raise OSError("Invalid native pipe wait result")
                if not kernel32.GetOverlappedResult(
                        self.handle.value, ctypes.byref(self.overlapped),
                        ctypes.byref(self.count), False):
                    error = ctypes.get_last_error()
                    if _terminal_completion(error):
                        self.settled = True
                    raise ctypes.WinError(error)
                self.settled = True
            self.result = (self.count.value if self.write
                           else self.buffer.raw[:self.count.value])
        except BaseException as error:
            if not self.settled:
                try:
                    self._drain()
                except BaseException as drain_error:
                    self.error = drain_error
                    _retain_operation(self)
                    return
            self.error = error
        finally:
            if self.settled:
                self.event.close()
                self.handle.close()


def _operation(handle, write, data, deadline):
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise NotSubmitted("Native pipe deadline expired before admission")
    if not _NATIVE_IO_SLOTS.acquire(timeout=remaining):
        raise NotSubmitted("Native pipe deadline expired before admission")
    try:
        with _NATIVE_IO_STATE:
            disabled = _NATIVE_IO_DISABLED
        if disabled:
            raise OSError("Native context I/O is disabled after unconfirmed cancellation")
        owner = _Operation(handle, write, data, deadline)
    except BaseException:
        _NATIVE_IO_SLOTS.release()
        raise
    worker = threading.Thread(target=owner.entry, name="runyte-context-native-io")
    interruption = None
    try:
        worker.start()
        while not owner.completed.is_set():
            owner.completed.wait()
    except BaseException as error:
        interruption = error
        try:
            owner.request_cancel()
        except BaseException:
            pass
        # Thread.start can be interrupted after native creation but before its
        # private bookkeeping completes. Trust only the owner's handshake.
        handshake_deadline = time.monotonic() + 1.0
        while not owner.started.is_set() and time.monotonic() < handshake_deadline:
            try:
                owner.started.wait(min(.05, handshake_deadline - time.monotonic()))
            except BaseException:
                pass
        if not owner.started.is_set():
            _retain_operation(owner)
            raise interruption
        while not owner.completed.is_set():
            try:
                owner.completed.wait()
            except BaseException:
                try:
                    owner.request_cancel()
                except BaseException:
                    pass
    if interruption is not None:
        raise interruption
    if owner.error is not None:
        raise owner.error
    return owner.result


def run_discovery(argv, timeout, maximum):
    """Run discovery through an owned overlapped pipe with one absolute deadline."""
    deadline = time.monotonic() + timeout
    address = rf"\\.\pipe\runyte-context-discovery-{os.getpid()}-{secrets.token_hex(16)}"
    reader = _Handle(kernel32.CreateNamedPipeW(
        address, PIPE_ACCESS_INBOUND | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
        PIPE_REJECT_REMOTE_CLIENTS, 1, 65536, 65536, 0, None))
    writer = None
    output = None
    child = None
    try:
        security = SECURITY_ATTRIBUTES(ctypes.sizeof(SECURITY_ATTRIBUTES), None, True)
        writer = _Handle(kernel32.CreateFileW(address, GENERIC_WRITE, 0,
                                              ctypes.byref(security), OPEN_EXISTING,
                                              SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                                              None))
        descriptor = msvcrt.open_osfhandle(writer.value, os.O_BINARY)
        writer.value = None
        output = os.fdopen(descriptor, "wb", buffering=0)
        child = subprocess.Popen(argv, stdout=output, stderr=subprocess.DEVNULL,
                                 close_fds=True, creationflags=CREATE_NO_WINDOW)
        output.close()
        output = None
        data = bytearray()
        while len(data) <= maximum:
            chunk = _operation(reader.value, False,
                               min(65536, maximum + 1 - len(data)), deadline)
            if not chunk:
                break
            data.extend(chunk)
        if len(data) > maximum:
            child.kill()
            child.wait()
            return bytes(data), 0
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError()
        try:
            status = child.wait(timeout=remaining)
        except subprocess.TimeoutExpired:
            raise TimeoutError() from None
        return bytes(data), status
    except BaseException:
        if child is not None and child.poll() is None:
            child.kill()
            child.wait()
        raise
    finally:
        if output is not None:
            output.close()
        if writer is not None:
            writer.close()
        reader.close()


class Pipe:
    def __init__(self, record, timeout):
        validate_record(record)
        self.deadline = None
        self.handle = None
        self.process = None
        deadline = time.monotonic() + timeout
        endpoint = record["endpoint"]
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError()
            value = kernel32.CreateFileW(endpoint, GENERIC_READ | GENERIC_WRITE, 0, None,
                                         OPEN_EXISTING, FILE_FLAG_OVERLAPPED |
                                         SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, None)
            if value != INVALID_HANDLE_VALUE:
                break
            error = ctypes.get_last_error()
            if error not in (ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY):
                raise ctypes.WinError(error)
            milliseconds = max(1, min(100, int(remaining * 1000)))
            if error == ERROR_PIPE_BUSY:
                kernel32.WaitNamedPipeW(endpoint, milliseconds)
            else:
                time.sleep(min(.01, remaining))
        self.handle = _Handle(value)
        try:
            if kernel32.GetFileType(self.handle.value) != FILE_TYPE_PIPE:
                raise OSError("Context endpoint is not a named pipe")
            server_pid = wintypes.ULONG()
            if not kernel32.GetNamedPipeServerProcessId(self.handle.value,
                                                        ctypes.byref(server_pid)):
                raise _failed("Cannot inspect context pipe server")
            if server_pid.value != record["pid"]:
                raise PermissionError("Context pipe server identity changed")
            self.process = _process(record["pid"], record["creation_time"])
        except BaseException:
            self.close()
            raise

    def settimeout(self, timeout):
        self.deadline = time.monotonic() + timeout

    def _deadline(self):
        if self.deadline is None:
            raise TimeoutError()
        return self.deadline

    def sendall(self, data):
        view = memoryview(data)
        transmitted = 0
        while view:
            try:
                written = _operation(self.handle.value, True, view.tobytes(), self._deadline())
            except NotSubmitted:
                if transmitted:
                    raise TimeoutError("Context frame was only partially sent") from None
                raise
            if not written:
                raise ConnectionError("Context pipe closed during write")
            transmitted += written
            view = view[written:]

    def recv(self, maximum):
        return _operation(self.handle.value, False, maximum, self._deadline())

    def alive(self):
        if not self.handle or not self.process:
            return False
        available = wintypes.DWORD()
        if not kernel32.PeekNamedPipe(self.handle.value, None, 0, None,
                                      ctypes.byref(available), None):
            return False
        return (not available.value
                and kernel32.WaitForSingleObject(self.process.value, 0) == WAIT_TIMEOUT)

    def close(self):
        if self.handle:
            kernel32.CancelIoEx(self.handle.value, None)
            self.handle.close()
            self.handle = None
        if self.process:
            self.process.close()
            self.process = None
