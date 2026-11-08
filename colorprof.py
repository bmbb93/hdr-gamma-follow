"""mscms の Windows 10 2004+ ディスプレイ用カラープロファイル API の薄いラッパー。"""
import ctypes as C
from ctypes import wintypes as W

from probe_sdr import LUID

mscms = C.WinDLL("mscms")

SCOPE_SYSTEM, SCOPE_USER = 0, 1
CPT_ICC = 0
CPST_STANDARD_DISPLAY_COLOR_MODE, CPST_EXTENDED_DISPLAY_COLOR_MODE = 7, 8

HR = C.HRESULT
mscms.ColorProfileAddDisplayAssociation.argtypes = [C.c_int, W.LPCWSTR, LUID, W.UINT, W.BOOL, W.BOOL]
mscms.ColorProfileAddDisplayAssociation.restype = HR
mscms.ColorProfileRemoveDisplayAssociation.argtypes = [C.c_int, W.LPCWSTR, LUID, W.UINT, W.BOOL]
mscms.ColorProfileRemoveDisplayAssociation.restype = HR
mscms.ColorProfileSetDisplayDefaultAssociation.argtypes = [C.c_int, W.LPCWSTR, C.c_int, C.c_int, LUID, W.UINT]
mscms.ColorProfileSetDisplayDefaultAssociation.restype = HR
mscms.ColorProfileGetDisplayDefault.argtypes = [C.c_int, LUID, W.UINT, C.c_int, C.c_int, C.POINTER(W.LPWSTR)]
mscms.ColorProfileGetDisplayDefault.restype = HR
mscms.ColorProfileGetDisplayList.argtypes = [C.c_int, LUID, W.UINT, C.POINTER(C.POINTER(W.LPWSTR)), C.POINTER(W.DWORD)]
mscms.ColorProfileGetDisplayList.restype = HR
mscms.ColorProfileGetDisplayUserScope.argtypes = [LUID, W.UINT, C.POINTER(C.c_int)]
mscms.ColorProfileGetDisplayUserScope.restype = HR
mscms.InstallColorProfileW.argtypes = [W.LPCWSTR, W.LPCWSTR]
mscms.InstallColorProfileW.restype = W.BOOL
mscms.GetColorDirectoryW.argtypes = [W.LPCWSTR, W.LPWSTR, C.POINTER(W.DWORD)]
mscms.GetColorDirectoryW.restype = W.BOOL

LocalFree = C.windll.kernel32.LocalFree


def user_scope(adapter: LUID, source: int) -> int:
    s = C.c_int()
    mscms.ColorProfileGetDisplayUserScope(adapter, source, C.byref(s))
    return s.value


def get_default(scope: int, adapter: LUID, source: int, hdr: bool = True) -> str | None:
    p = W.LPWSTR()
    sub = CPST_EXTENDED_DISPLAY_COLOR_MODE if hdr else CPST_STANDARD_DISPLAY_COLOR_MODE
    try:
        mscms.ColorProfileGetDisplayDefault(scope, adapter, source, CPT_ICC, sub, C.byref(p))
    except OSError:
        return None
    v = p.value
    LocalFree(p)
    return v


def get_list(scope: int, adapter: LUID, source: int) -> list[str]:
    arr, n = C.POINTER(W.LPWSTR)(), W.DWORD()
    mscms.ColorProfileGetDisplayList(scope, adapter, source, C.byref(arr), C.byref(n))
    v = [arr[i] for i in range(n.value)]
    LocalFree(arr)
    return v


def color_dir() -> str:
    buf, n = C.create_unicode_buffer(520), W.DWORD(1040)
    mscms.GetColorDirectoryW(None, buf, C.byref(n))
    return buf.value


def install(path: str) -> None:
    if not mscms.InstallColorProfileW(None, path):
        raise C.WinError()


def associate_hdr(scope: int, name: str, adapter: LUID, source: int) -> None:
    """HDR (advanced color) プロファイルとして関連付け、既定にする。"""
    mscms.ColorProfileAddDisplayAssociation(scope, name, adapter, source, True, True)


def set_default_hdr(scope: int, name: str, adapter: LUID, source: int) -> None:
    mscms.ColorProfileSetDisplayDefaultAssociation(scope, name, CPT_ICC, CPST_EXTENDED_DISPLAY_COLOR_MODE,
                                                   adapter, source)


def dissociate_hdr(scope: int, name: str, adapter: LUID, source: int) -> None:
    mscms.ColorProfileRemoveDisplayAssociation(scope, name, adapter, source, True)


if __name__ == "__main__":
    from probe_sdr import paths
    print("color dir:", color_dir())
    for p in paths():
        a, s = p.sourceInfo.adapterId, p.sourceInfo.id
        sc = user_scope(a, s)
        print(f"source {s}: user scope={'USER' if sc else 'SYSTEM'}")
        for scope in (SCOPE_USER, SCOPE_SYSTEM):
            print(f"  [{'USER' if scope else 'SYSTEM'}] HDR default: {get_default(scope, a, s, True)}"
                  f" | SDR default: {get_default(scope, a, s, False)}")
            print(f"    list: {get_list(scope, a, s)}")
