# Run explicitly after extracting Serein to its final folder. No administrator rights required.
# Creates only the current user's Serein Start Menu shortcut; does not enable OS alerts in Serein.
param([switch]$Remove, [switch]$Force)
$ErrorActionPreference = 'Stop'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Serein.lnk'
if ($Remove) {
    if (Test-Path -LiteralPath $shortcut) { Remove-Item -LiteralPath $shortcut }
    return
}
$executable = Join-Path $PSScriptRoot 'serein.exe'
if (!(Test-Path -LiteralPath $executable)) { throw 'Keep this script next to serein.exe in its final folder.' }
if ((Test-Path -LiteralPath $shortcut) -and !$Force) { throw 'Serein.lnk already exists. Remove it explicitly with -Remove before replacing it.' }
if ($Force -and (Test-Path -LiteralPath $shortcut)) { Remove-Item -LiteralPath $shortcut -Force }

# A desktop toast requires a Start Menu shortcut carrying the same AppUserModelID as the notifier.
# https://learn.microsoft.com/windows/win32/shell/enable-desktop-toast-with-appusermodelid
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class SereinShortcut {
    [StructLayout(LayoutKind.Sequential)] struct PropertyKey {
        public Guid format; public uint id;
    }
    [StructLayout(LayoutKind.Explicit)] struct PropVariant {
        [FieldOffset(0)] public ushort type;
        [FieldOffset(8)] public IntPtr value;
        [FieldOffset(16)] private IntPtr padding;
    }
    [ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IPropertyStore {
        void GetCount(out uint count);
        void GetAt(uint index, out PropertyKey key);
        void GetValue(ref PropertyKey key, out PropVariant value);
        void SetValue(ref PropertyKey key, ref PropVariant value);
        void Commit();
    }
    [DllImport("shell32.dll", CharSet=CharSet.Unicode, PreserveSig=false)]
    static extern void SHGetPropertyStoreFromParsingName(string path, IntPtr bindContext, uint flags,
        ref Guid iid, [MarshalAs(UnmanagedType.Interface)] out IPropertyStore store);
    public static void SetAppId(string path) {
        Guid iid = typeof(IPropertyStore).GUID;
        IPropertyStore store;
        SHGetPropertyStoreFromParsingName(path, IntPtr.Zero, 2, ref iid, out store);
        PropertyKey key = new PropertyKey { format = new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"), id = 5 };
        PropVariant value = new PropVariant { type = 31, value = Marshal.StringToCoTaskMemUni("cz.viceverse.serein") };
        try { store.SetValue(ref key, ref value); store.Commit(); }
        finally { Marshal.FreeCoTaskMem(value.value); Marshal.FinalReleaseComObject(store); }
    }
}
'@
$shell = New-Object -ComObject WScript.Shell
try {
    $link = $shell.CreateShortcut($shortcut)
    $link.TargetPath = $executable
    $link.WorkingDirectory = $PSScriptRoot
    $link.Description = 'Serein'
    $link.Save()
    [SereinShortcut]::SetAppId($shortcut)
} catch {
    if (Test-Path -LiteralPath $shortcut) { Remove-Item -LiteralPath $shortcut }
    throw
} finally {
    if ($null -ne $link) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($link) }
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
}
Write-Output 'Serein Start Menu shortcut registered. Enable system notifications separately in Serein for each session.'
