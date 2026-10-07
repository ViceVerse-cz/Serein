param([int]$ProcessId, [string]$OutputPath)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
if (-not ('PreviewWindow' -as [type])) {
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class PreviewWindow {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    private delegate bool WindowCallback(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(WindowCallback callback, IntPtr data);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")] private static extern int GetWindowTextLength(IntPtr window);
    public static IntPtr FindOwnedWindow(int processId) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((window, data) => {
            uint owner; GetWindowThreadProcessId(window, out owner);
            if (owner == processId && GetWindowTextLength(window) > 0) { result = window; return false; }
            return true;
        }, IntPtr.Zero);
        return result;
    }
}
'@
}
[PreviewWindow]::SetProcessDPIAware() | Out-Null
$preview = Get-Process -Id $ProcessId
if ($preview.ProcessName -ne 'profile_preview') { throw 'Capture only the synthetic profile_preview process.' }
$handle = $preview.MainWindowHandle
if ($handle -eq [IntPtr]::Zero) { $handle = [PreviewWindow]::FindOwnedWindow($ProcessId) }
if ($handle -eq [IntPtr]::Zero) { throw 'Preview has no window.' }
[PreviewWindow]::ShowWindow($handle, 9) | Out-Null
[PreviewWindow]::SetForegroundWindow($handle) | Out-Null
Start-Sleep -Milliseconds 500
if ([PreviewWindow]::GetForegroundWindow() -ne $handle) {
    Add-Type -AssemblyName UIAutomationClient
    [System.Windows.Automation.AutomationElement]::FromHandle($handle).SetFocus()
    Start-Sleep -Milliseconds 500
}
if ([PreviewWindow]::GetForegroundWindow() -ne $handle) { throw 'Preview did not become foreground.' }
$rect = New-Object PreviewWindow+Rect
if (-not [PreviewWindow]::GetWindowRect($handle, [ref]$rect)) { throw 'Cannot read preview window bounds.' }
$bitmap = New-Object System.Drawing.Bitmap(($rect.Right-$rect.Left), ($rect.Bottom-$rect.Top))
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
    $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
    $bitmap.Save($OutputPath, [System.Drawing.Imaging.ImageFormat]::Png)
    [pscustomobject]@{ ProcessId=$ProcessId; Width=$bitmap.Width; Height=$bitmap.Height; Dpi=[PreviewWindow]::GetDpiForWindow($handle); Path=$OutputPath } | ConvertTo-Json -Compress
} finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}
