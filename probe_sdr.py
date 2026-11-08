"""接続中ディスプレイの SDR 白レベルと HDR 状態を表示する（読み取りのみ）。"""
import ctypes as C
from ctypes import wintypes as W

user32 = C.WinDLL("user32")

QDC_ONLY_ACTIVE_PATHS = 2
GET_SOURCE_NAME = 1
GET_TARGET_NAME = 2
GET_ADVANCED_COLOR_INFO = 9
GET_SDR_WHITE_LEVEL = 11


class LUID(C.Structure):
    _fields_ = [("LowPart", W.DWORD), ("HighPart", W.LONG)]


class PATH_SOURCE(C.Structure):
    _fields_ = [("adapterId", LUID), ("id", W.UINT), ("modeInfoIdx", W.UINT), ("statusFlags", W.UINT)]


class RATIONAL(C.Structure):
    _fields_ = [("Numerator", W.UINT), ("Denominator", W.UINT)]


class PATH_TARGET(C.Structure):
    _fields_ = [("adapterId", LUID), ("id", W.UINT), ("modeInfoIdx", W.UINT), ("outputTechnology", W.UINT),
                ("rotation", W.UINT), ("scaling", W.UINT), ("refreshRate", RATIONAL),
                ("scanLineOrdering", W.UINT), ("targetAvailable", W.BOOL), ("statusFlags", W.UINT)]


class PATH_INFO(C.Structure):
    _fields_ = [("sourceInfo", PATH_SOURCE), ("targetInfo", PATH_TARGET), ("flags", W.UINT)]


class MODE_INFO(C.Structure):
    _fields_ = [("infoType", W.UINT), ("id", W.UINT), ("adapterId", LUID), ("data", C.c_byte * 48)]


class HEADER(C.Structure):
    _fields_ = [("type", W.UINT), ("size", W.UINT), ("adapterId", LUID), ("id", W.UINT)]


class SDR_WHITE(C.Structure):
    _fields_ = [("header", HEADER), ("SDRWhiteLevel", W.ULONG)]


class ADV_COLOR(C.Structure):
    _fields_ = [("header", HEADER), ("value", W.UINT), ("colorEncoding", W.UINT), ("bitsPerColorChannel", W.UINT)]


class SOURCE_NAME(C.Structure):
    _fields_ = [("header", HEADER), ("viewGdiDeviceName", W.WCHAR * 32)]


class TARGET_NAME(C.Structure):
    _fields_ = [("header", HEADER), ("flags", W.UINT), ("outputTechnology", W.UINT), ("edidManufactureId", W.USHORT),
                ("edidProductCodeId", W.USHORT), ("connectorInstance", W.UINT),
                ("monitorFriendlyDeviceName", W.WCHAR * 64), ("monitorDevicePath", W.WCHAR * 128)]


def query(t, struct, adapter, id_):
    s = struct()
    s.header.type, s.header.size, s.header.adapterId, s.header.id = t, C.sizeof(struct), adapter, id_
    rc = user32.DisplayConfigGetDeviceInfo(C.byref(s))
    if rc:
        raise OSError(rc)
    return s


def paths():
    np_, nm = W.UINT(), W.UINT()
    user32.GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, C.byref(np_), C.byref(nm))
    P, M = (PATH_INFO * np_.value)(), (MODE_INFO * nm.value)()
    rc = user32.QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, C.byref(np_), P, C.byref(nm), M, None)
    if rc:
        raise OSError(rc)
    return list(P[: np_.value])


if __name__ == "__main__":
    for p in paths():
        src, tgt = p.sourceInfo, p.targetInfo
        name = query(GET_SOURCE_NAME, SOURCE_NAME, src.adapterId, src.id).viewGdiDeviceName
        tn = query(GET_TARGET_NAME, TARGET_NAME, tgt.adapterId, tgt.id)
        adv = query(GET_ADVANCED_COLOR_INFO, ADV_COLOR, tgt.adapterId, tgt.id)
        sdr = query(GET_SDR_WHITE_LEVEL, SDR_WHITE, tgt.adapterId, tgt.id).SDRWhiteLevel
        print(f"{name}  {tn.monitorFriendlyDeviceName!r}")
        print(f"  adapter LUID={tgt.adapterId.HighPart:08x}:{tgt.adapterId.LowPart:08x} sourceId={src.id} targetId={tgt.id}")
        print(f"  advancedColor flags=0x{adv.value:x} (supported={adv.value & 1}, enabled={(adv.value >> 1) & 1})")
        print(f"  SDRWhiteLevel raw={sdr} -> {sdr * 80 / 1000:.1f} nits")
