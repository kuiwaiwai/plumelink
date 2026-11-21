param([Parameter(Mandatory=$true)][int]$TargetProcessId, [ValidateSet('state','close','minimize','restore','offscreen','resize','tiny')][string]$Action='state')
$ErrorActionPreference='Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public class PlumeLinkWindowTest {
 public delegate bool EnumProc(IntPtr h,IntPtr l);
 [DllImport("user32.dll")]public static extern bool EnumWindows(EnumProc p,IntPtr l);
 [DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)]public static extern int GetWindowText(IntPtr h,StringBuilder s,int n);
 [DllImport("user32.dll")]public static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll")]public static extern bool IsIconic(IntPtr h);
 [DllImport("user32.dll")]public static extern bool IsZoomed(IntPtr h);
 [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
 [DllImport("user32.dll")]public static extern bool ClientToScreen(IntPtr h,ref Point point);
 [DllImport("user32.dll")]public static extern bool GetWindowRect(IntPtr h,out Rect rect);
 [DllImport("user32.dll")]public static extern bool GetClientRect(IntPtr h,out Rect rect);
 [DllImport("user32.dll")]public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int width,int height,uint flags);
 [DllImport("user32.dll")]public static extern IntPtr MonitorFromWindow(IntPtr h,uint flags);
 public static bool ClientFillsWindow(IntPtr h){Rect outer,client;if(!GetWindowRect(h,out outer)||!GetClientRect(h,out client))return false;var top=new Point();var bottom=new Point{X=client.Right,Y=client.Bottom};if(!ClientToScreen(h,ref top)||!ClientToScreen(h,ref bottom))return false;return top.X==outer.Left&&top.Y==outer.Top&&bottom.X==outer.Right&&bottom.Y==outer.Bottom;}
 [DllImport("user32.dll")]public static extern uint GetDpiForWindow(IntPtr h);
 [DllImport("dwmapi.dll")]public static extern int DwmGetWindowAttribute(IntPtr h,int attribute,out int value,int size);
 public static bool CompositorRenderingEnabled(IntPtr h){int value;return DwmGetWindowAttribute(h,1,out value,4)==0&&value!=0;}
 public static int TopInset(IntPtr h){var point=new Point();Rect rect;ClientToScreen(h,ref point);GetWindowRect(h,out rect);return point.Y-rect.Top;}
 [DllImport("user32.dll")]public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
 [DllImport("user32.dll")]public static extern bool ShowWindow(IntPtr h,int command);
 public static IntPtr Find(int pid){IntPtr found=IntPtr.Zero;EnumWindows((h,l)=>{uint p;GetWindowThreadProcessId(h,out p);var s=new StringBuilder(512);GetWindowText(h,s,512);if(p==pid && s.ToString().Contains("PlumeLink")){found=h;return false;}return true;},IntPtr.Zero);return found;}
}
'@
$windowHandle=[PlumeLinkWindowTest]::Find($TargetProcessId)
if($windowHandle -eq [IntPtr]::Zero){throw 'Owned PlumeLink window not found'}
if($Action -eq 'close'){[PlumeLinkWindowTest]::PostMessage($windowHandle,0x10,[IntPtr]::Zero,[IntPtr]::Zero)|Out-Null}
if($Action -eq 'minimize'){[PlumeLinkWindowTest]::PostMessage($windowHandle,0x112,0xF020,[IntPtr]::Zero)|Out-Null}
if($Action -eq 'restore'){[PlumeLinkWindowTest]::PostMessage($windowHandle,0x112,0xF120,[IntPtr]::Zero)|Out-Null}
# Inject an unusable client size without activating the test window. Normal
# interactive resizing enforces a minimum before this recovery path is reached.
if($Action -eq 'tiny'){[PlumeLinkWindowTest]::SetWindowPos($windowHandle,[IntPtr]::Zero,0,0,120,80,0x416)|Out-Null}
if($Action -eq 'resize'){
    $before=New-Object 'PlumeLinkWindowTest+Rect'
    [PlumeLinkWindowTest]::GetWindowRect($windowHandle,[ref]$before)|Out-Null
    [PlumeLinkWindowTest]::SetWindowPos($windowHandle,[IntPtr]::Zero,0,0,($before.Right-$before.Left-48),($before.Bottom-$before.Top-28),0x16)|Out-Null
}
if($Action -eq 'offscreen'){[PlumeLinkWindowTest]::SetWindowPos($windowHandle,[IntPtr]::Zero,-30000,-30000,0,0,0x15)|Out-Null}
$rect=New-Object 'PlumeLinkWindowTest+Rect'
[PlumeLinkWindowTest]::GetClientRect($windowHandle,[ref]$rect)|Out-Null
@{handle=$windowHandle.ToInt64();width=$rect.Right;height=$rect.Bottom;onMonitor=([PlumeLinkWindowTest]::MonitorFromWindow($windowHandle,0) -ne [IntPtr]::Zero);visible=[PlumeLinkWindowTest]::IsWindowVisible($windowHandle);minimized=[PlumeLinkWindowTest]::IsIconic($windowHandle);maximized=[PlumeLinkWindowTest]::IsZoomed($windowHandle);topInset=[PlumeLinkWindowTest]::TopInset($windowHandle);clientFillsWindow=[PlumeLinkWindowTest]::ClientFillsWindow($windowHandle);compositorRenderingEnabled=[PlumeLinkWindowTest]::CompositorRenderingEnabled($windowHandle);dpi=[PlumeLinkWindowTest]::GetDpiForWindow($windowHandle)} | ConvertTo-Json -Compress
