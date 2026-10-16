param([Parameter(Mandatory)][string]$Executable)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force out | Out-Null
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Pointer {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
}
'@
$scope = [System.Windows.Automation.TreeScope]
$element = [System.Windows.Automation.AutomationElement]
$invoke = [System.Windows.Automation.InvokePattern]::Pattern

function Find-Element($Root, $Name) {
    $condition = [System.Windows.Automation.PropertyCondition]::new($element::NameProperty, $Name)
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        $found = $Root.FindFirst($scope::Descendants, $condition)
        if ($found) { return $found }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Menu item '$Name' is missing"
}

function Save-Window($Window) {
    $bounds = $Window.Current.BoundingRectangle
    $image = [System.Drawing.Bitmap]::new([int]$bounds.Width, [int]$bounds.Height)
    $graphics = [System.Drawing.Graphics]::FromImage($image)
    try {
        $graphics.CopyFromScreen([int]$bounds.X, [int]$bounds.Y, 0, 0, $image.Size)
        $image.Save("$PWD/out/windows-menu.png")
    } finally {
        $graphics.Dispose()
        $image.Dispose()
    }
}

$process = Start-Process (Resolve-Path $Executable) -PassThru -RedirectStandardError out/windows-startup.log
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        $process.Refresh()
        if ($process.HasExited) { throw "hachistep exited with code $($process.ExitCode)" }
        if ($process.MainWindowHandle -ne 0) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($process.MainWindowHandle -eq 0) { throw 'hachistep did not open a window' }
    $window = $element::FromHandle($process.MainWindowHandle)
    $condition = [System.Windows.Automation.PropertyCondition]::new($element::ProcessIdProperty, $process.Id)
    $before = $element::RootElement.FindAll($scope::Children, $condition).Count
    Save-Window $window
    $file = Find-Element $window 'File'
    $file.GetCurrentPattern($invoke).Invoke()
    $new = Find-Element $window 'New Window'
    Save-Window $window
    $point = $new.GetClickablePoint()
    [Pointer]::SetCursorPos([int]$point.X, [int]$point.Y) | Out-Null
    [Pointer]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
    [Pointer]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        $windows = $element::RootElement.FindAll($scope::Children, $condition)
        if ($windows.Count -gt $before) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($windows.Count -le $before) { throw 'File > New Window did not open a second player' }
    Write-Output 'File menu opened and New Window created a second player'
} finally {
    if (!$process.HasExited) { $process.Kill($true) }
}
