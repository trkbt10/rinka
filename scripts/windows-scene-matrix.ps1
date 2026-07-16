[CmdletBinding()]
param(
    [string]$Binary = "target\debug\rinka-explorer.exe",
    [string]$OutputDirectory = "target\windows-scene-matrix"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName Accessibility
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationClientsideProviders
Add-Type -AssemblyName UIAutomationTypes

Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public static class RinkaNativeProbe
{
    public delegate bool EnumWindowsCallback(IntPtr hwnd, IntPtr value);
    public delegate bool EnumChildWindowsCallback(IntPtr hwnd, IntPtr value);

    [StructLayout(LayoutKind.Sequential)]
    public struct Rect
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsCallback callback, IntPtr value);

    [DllImport("user32.dll")]
    public static extern bool EnumChildWindows(
        IntPtr parent,
        EnumChildWindowsCallback callback,
        IntPtr value
    );

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hwnd);

    [DllImport("user32.dll")]
    public static extern bool IsWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    public static extern bool IsWindowEnabled(IntPtr hwnd);

    [DllImport("user32.dll")]
    public static extern IntPtr GetWindow(IntPtr hwnd, uint command);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindowEx(
        IntPtr parent,
        IntPtr after,
        string className,
        string windowName
    );

    [DllImport("user32.dll", EntryPoint = "PostMessageW", SetLastError = true)]
    public static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", EntryPoint = "SendMessageW")]
    public static extern IntPtr SendMessageW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);

    [DllImport("oleacc.dll")]
    private static extern int AccessibleObjectFromWindow(
        IntPtr hwnd,
        uint objectId,
        ref Guid interfaceId,
        [MarshalAs(UnmanagedType.Interface)] out object accessible
    );

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassName(IntPtr hwnd, char[] value, int maximum);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);

    [DllImport("user32.dll")]
    public static extern bool GetClientRect(IntPtr hwnd, out Rect rect);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool SetWindowPos(
        IntPtr hwnd,
        IntPtr insertAfter,
        int x,
        int y,
        int width,
        int height,
        uint flags
    );

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    public static extern uint GetDpiForWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    public static extern IntPtr GetWindowDpiAwarenessContext(IntPtr hwnd);

    [DllImport("user32.dll")]
    public static extern bool AreDpiAwarenessContextsEqual(IntPtr first, IntPtr second);

    public static IntPtr[] ProcessWindows(uint targetProcessId)
    {
        List<IntPtr> result = new List<IntPtr>();
        EnumWindows(delegate (IntPtr hwnd, IntPtr value)
        {
            uint processId;
            GetWindowThreadProcessId(hwnd, out processId);
            if (processId == targetProcessId && IsWindowVisible(hwnd))
            {
                result.Add(hwnd);
            }
            return true;
        }, IntPtr.Zero);
        return result.ToArray();
    }

    public static IntPtr[] DescendantWindows(IntPtr parent)
    {
        List<IntPtr> result = new List<IntPtr>();
        EnumChildWindows(parent, delegate (IntPtr hwnd, IntPtr value)
        {
            result.Add(hwnd);
            return true;
        }, IntPtr.Zero);
        return result.ToArray();
    }

    public static string AccessibleName(IntPtr hwnd)
    {
        Guid interfaceId = typeof(Accessibility.IAccessible).GUID;
        object value;
        int result = AccessibleObjectFromWindow(hwnd, 0xFFFFFFFC, ref interfaceId, out value);
        if (result < 0 || value == null)
        {
            return "";
        }
        try
        {
            Accessibility.IAccessible accessible = (Accessibility.IAccessible)value;
            return accessible.get_accName(0) ?? "";
        }
        catch
        {
            return "";
        }
    }

    public static string ClassName(IntPtr hwnd)
    {
        char[] storage = new char[256];
        int length = GetClassName(hwnd, storage, storage.Length);
        return new string(storage, 0, length);
    }
}
"@ -ReferencedAssemblies Accessibility

function Get-WindowRectangle {
    param([IntPtr]$Handle)
    $rectangle = New-Object RinkaNativeProbe+Rect
    if (-not [RinkaNativeProbe]::GetWindowRect($Handle, [ref]$rectangle)) {
        throw "GetWindowRect failed for $Handle"
    }
    [ordered]@{
        left = $rectangle.Left
        top = $rectangle.Top
        right = $rectangle.Right
        bottom = $rectangle.Bottom
        width = $rectangle.Right - $rectangle.Left
        height = $rectangle.Bottom - $rectangle.Top
    }
}

function Get-ClientRectangle {
    param([IntPtr]$Handle)
    $rectangle = New-Object RinkaNativeProbe+Rect
    if (-not [RinkaNativeProbe]::GetClientRect($Handle, [ref]$rectangle)) {
        throw "GetClientRect failed for $Handle"
    }
    [ordered]@{
        width = $rectangle.Right - $rectangle.Left
        height = $rectangle.Bottom - $rectangle.Top
    }
}

function Get-ContentRectangle {
    param([IntPtr]$Handle)
    $contentHandle = [RinkaNativeProbe]::FindWindowEx(
        $Handle,
        [IntPtr]::Zero,
        "STATIC",
        "Rinka content root"
    )
    if ($contentHandle -eq [IntPtr]::Zero) {
        $frame = Get-WindowRectangle -Handle $Handle
        $client = Get-ClientRectangle -Handle $Handle
        return [ordered]@{
            left = $frame.left
            top = $frame.top
            right = $frame.left + $client.width
            bottom = $frame.top + $client.height
            width = $client.width
            height = $client.height
        }
    }
    return Get-WindowRectangle -Handle $contentHandle
}

function Set-ContentRectangle {
    param(
        [IntPtr]$Handle,
        [int]$Width,
        [int]$Height,
        [int]$NativeHeightAdjustment = 0
    )
    $frame = Get-WindowRectangle -Handle $Handle
    $content = Get-ContentRectangle -Handle $Handle
    $nativeHeight = $Height + $NativeHeightAdjustment
    $outerWidth = $Width + ($frame.width - $content.width)
    $outerHeight = $nativeHeight + ($frame.height - $content.height)
    if (-not [RinkaNativeProbe]::SetWindowPos(
        $Handle,
        [IntPtr]::Zero,
        0,
        0,
        $outerWidth,
        $outerHeight,
        0x0016
    )) {
        throw "SetWindowPos failed while sizing the client area"
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        Start-Sleep -Milliseconds 100
        $content = Get-ContentRectangle -Handle $Handle
        if ($content.width -eq $Width -and $content.height -eq $nativeHeight) {
            if ($NativeHeightAdjustment -eq 0) {
                return $content
            }
            return [ordered]@{
                left = $content.left
                top = $content.top
                right = $content.right
                bottom = $content.bottom - $NativeHeightAdjustment
                width = $Width
                height = $Height
                native_height = $content.height
            }
        }
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Content area is $($content.width)x$($content.height), expected ${Width}x${nativeHeight} native pixels for a ${Width}x${Height} content contract"
}

function Wait-ProcessWindows {
    param([System.Diagnostics.Process]$Process)
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    while ([DateTime]::UtcNow -lt $deadline) {
        $Process.Refresh()
        if ($Process.HasExited) {
            throw "Explorer process exited before its native window became ready"
        }
        $windows = [RinkaNativeProbe]::ProcessWindows([uint32]$Process.Id)
        if ($windows.Count -gt 0) {
            return $windows
        }
        Start-Sleep -Milliseconds 250
    }
    throw "Explorer process did not expose a visible window within 45 seconds"
}

function Get-AutomationRecord {
    param([IntPtr]$Handle)
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($Handle)
    $elements = $root.FindAll(
        [System.Windows.Automation.TreeScope]::Subtree,
        [System.Windows.Automation.Condition]::TrueCondition
    )
    $records = [System.Collections.Generic.List[object]]::new()
    foreach ($element in $elements) {
        try {
            $rectangle = $element.Current.BoundingRectangle
            $records.Add([ordered]@{
                name = $element.Current.Name
                automation_id = $element.Current.AutomationId
                class_name = $element.Current.ClassName
                control_type = $element.Current.ControlType.ProgrammaticName
                enabled = $element.Current.IsEnabled
                keyboard_focusable = $element.Current.IsKeyboardFocusable
                bounds = [ordered]@{
                    x = [Math]::Round($rectangle.X, 2)
                    y = [Math]::Round($rectangle.Y, 2)
                    width = [Math]::Round($rectangle.Width, 2)
                    height = [Math]::Round($rectangle.Height, 2)
                }
            })
        }
        catch {
            continue
        }
    }
    return $records
}

function Get-AccessibilityRecord {
    param([IntPtr]$Handle)
    $handles = [System.Collections.Generic.List[IntPtr]]::new()
    $handles.Add($Handle)
    foreach ($child in [RinkaNativeProbe]::DescendantWindows($Handle)) {
        $handles.Add($child)
    }
    $records = [System.Collections.Generic.List[object]]::new()
    foreach ($candidate in $handles) {
        if (-not [RinkaNativeProbe]::IsWindow($candidate)) {
            continue
        }
        $rectangle = Get-WindowRectangle -Handle $candidate
        $records.Add([ordered]@{
            hwnd = $candidate.ToInt64()
            name = [RinkaNativeProbe]::AccessibleName($candidate)
            class_name = [RinkaNativeProbe]::ClassName($candidate)
            enabled = [RinkaNativeProbe]::IsWindowEnabled($candidate)
            bounds = $rectangle
        })
    }
    return $records
}

function Assert-AutomationWithinFrame {
    param(
        [object[]]$Automation,
        [object]$Frame,
        [string]$WindowName
    )
    foreach ($element in $Automation) {
        $bounds = $element.bounds
        if ($bounds.width -le 0 -or $bounds.height -le 0) {
            continue
        }
        $right = $bounds.x + $bounds.width
        $bottom = $bounds.y + $bounds.height
        if ($bounds.x -lt ($Frame.left - 1) -or $bounds.y -lt ($Frame.top - 1) -or
            $right -gt ($Frame.right + 1) -or $bottom -gt ($Frame.bottom + 1)) {
            throw "UI Automation element '$($element.name)' is clipped outside $WindowName"
        }
    }
}

function Require-AccessibleNames {
    param(
        [object[]]$Records,
        [string[]]$Names,
        [string]$CaseName
    )
    $observed = @($Records | ForEach-Object { $_.name })
    foreach ($name in $Names) {
        if ($observed -notcontains $name) {
            throw "$CaseName is missing accessible name '$name'"
        }
    }
}

function Invoke-PaneCycles {
    param(
        [IntPtr]$Handle,
        [string]$ButtonName
    )
    $button = [IntPtr]::Zero
    foreach ($candidate in [RinkaNativeProbe]::DescendantWindows($Handle)) {
        if ([RinkaNativeProbe]::ClassName($candidate) -eq "Button" -and
            [RinkaNativeProbe]::AccessibleName($candidate) -eq $ButtonName) {
            $button = $candidate
            break
        }
    }
    if ($button -eq [IntPtr]::Zero) {
        throw "Required pane toggle '$ButtonName' is not visible to Active Accessibility"
    }
    $frames = [System.Collections.Generic.List[object]]::new()
    $frames.Add((Get-WindowRectangle -Handle $Handle))
    for ($cycle = 0; $cycle -lt 3; $cycle += 1) {
        $null = [RinkaNativeProbe]::SendMessageW(
            $button,
            0x00F5,
            [IntPtr]::Zero,
            [IntPtr]::Zero
        )
        Start-Sleep -Milliseconds 250
        $frames.Add((Get-WindowRectangle -Handle $Handle))
        $null = [RinkaNativeProbe]::SendMessageW(
            $button,
            0x00F5,
            [IntPtr]::Zero,
            [IntPtr]::Zero
        )
        Start-Sleep -Milliseconds 250
        $frames.Add((Get-WindowRectangle -Handle $Handle))
    }
    $first = $frames[0]
    foreach ($frame in $frames) {
        if ($frame.left -ne $first.left -or $frame.top -ne $first.top -or
            $frame.width -ne $first.width -or $frame.height -ne $first.height) {
            throw "Top-level frame changed while cycling '$ButtonName'"
        }
    }
    return $frames
}

function Invoke-AutomationPaneCycles {
    param(
        [IntPtr]$Handle,
        [string]$ButtonName
    )
    $invokeButton = {
        param([string]$Name)
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($Handle)
        $condition = New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::NameProperty,
            $Name
        )
        $button = $null
        $matches = $root.FindAll(
            [System.Windows.Automation.TreeScope]::Subtree,
            $condition
        )
        foreach ($candidate in $matches) {
            if ($candidate.Current.ControlType -eq
                [System.Windows.Automation.ControlType]::Button) {
                $button = $candidate
                break
            }
        }
        if ($null -eq $button -and $Name -eq "Details") {
            $overflowCondition = New-Object System.Windows.Automation.PropertyCondition(
                [System.Windows.Automation.AutomationElement]::NameProperty,
                "More options"
            )
            $overflow = $root.FindFirst(
                [System.Windows.Automation.TreeScope]::Subtree,
                $overflowCondition
            )
            if ($null -ne $overflow) {
                $overflowPattern = $overflow.GetCurrentPattern(
                    [System.Windows.Automation.InvokePattern]::Pattern
                )
                $overflowPattern.Invoke()
                Start-Sleep -Milliseconds 250
                $matches = [System.Windows.Automation.AutomationElement]::RootElement.FindAll(
                    [System.Windows.Automation.TreeScope]::Descendants,
                    $condition
                )
                foreach ($candidate in $matches) {
                    if ($candidate.Current.ControlType -eq
                        [System.Windows.Automation.ControlType]::Button) {
                        $button = $candidate
                        break
                    }
                }
            }
        }
        if ($null -eq $button) {
            throw "Required WinUI pane toggle '$Name' was not found"
        }
        $buttonPattern = $button.GetCurrentPattern(
            [System.Windows.Automation.InvokePattern]::Pattern
        )
        $buttonPattern.Invoke()
    }
    $frames = [System.Collections.Generic.List[object]]::new()
    $frames.Add((Get-WindowRectangle -Handle $Handle))
    for ($cycle = 0; $cycle -lt 3; $cycle += 1) {
        & $invokeButton $ButtonName
        Start-Sleep -Milliseconds 250
        $frames.Add((Get-WindowRectangle -Handle $Handle))
        & $invokeButton $ButtonName
        Start-Sleep -Milliseconds 250
        $frames.Add((Get-WindowRectangle -Handle $Handle))
    }
    $first = $frames[0]
    foreach ($frame in $frames) {
        if ($frame.left -ne $first.left -or $frame.top -ne $first.top -or
            $frame.width -ne $first.width -or $frame.height -ne $first.height) {
            throw "Top-level frame changed while cycling WinUI '$ButtonName'"
        }
    }
    return $frames
}

function Save-WindowCapture {
    param(
        [IntPtr]$Handle,
        [string]$Path
    )
    $rectangle = Get-WindowRectangle -Handle $Handle
    $bitmap = New-Object System.Drawing.Bitmap($rectangle.width, $rectangle.height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen(
            $rectangle.left,
            $rectangle.top,
            0,
            0,
            $bitmap.Size,
            [System.Drawing.CopyPixelOperation]::SourceCopy
        )
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

function Get-CaptureLuminance {
    param([string]$Path)
    $bitmap = [System.Drawing.Bitmap]::FromFile($Path)
    try {
        $stepX = [Math]::Max(1, [int]($bitmap.Width / 32))
        $stepY = [Math]::Max(1, [int]($bitmap.Height / 24))
        [double]$total = 0
        [int]$count = 0
        for ($y = [int]($stepY / 2); $y -lt $bitmap.Height; $y += $stepY) {
            for ($x = [int]($stepX / 2); $x -lt $bitmap.Width; $x += $stepX) {
                $pixel = $bitmap.GetPixel($x, $y)
                $total += 0.2126 * $pixel.R + 0.7152 * $pixel.G + 0.0722 * $pixel.B
                $count += 1
            }
        }
        return [Math]::Round($total / $count, 2)
    }
    finally {
        $bitmap.Dispose()
    }
}

function Close-PanelAndRequireMainWindow {
    param(
        [System.Diagnostics.Process]$Process,
        [IntPtr]$PanelHandle,
        [IntPtr]$MainHandle
    )
    if (-not [RinkaNativeProbe]::PostMessageW(
        $PanelHandle,
        0x0010,
        [IntPtr]::Zero,
        [IntPtr]::Zero
    )) {
        throw "Posting WM_CLOSE to the activity panel failed"
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([DateTime]::UtcNow -lt $deadline -and [RinkaNativeProbe]::IsWindow($PanelHandle)) {
        Start-Sleep -Milliseconds 100
    }
    if ([RinkaNativeProbe]::IsWindow($PanelHandle)) {
        throw "Activity panel did not close within five seconds"
    }
    $Process.Refresh()
    if ($Process.HasExited -or -not [RinkaNativeProbe]::IsWindow($MainHandle)) {
        throw "Closing the activity panel terminated its main window"
    }
}

function Stop-ProbeProcess {
    param([System.Diagnostics.Process]$Process)
    if ($Process.HasExited) {
        return
    }
    $null = $Process.CloseMainWindow()
    if (-not $Process.WaitForExit(3000)) {
        $Process.Kill()
        $Process.WaitForExit()
    }
}

function Start-ProbeCase {
    param(
        [string]$Scene,
        [string]$Appearance,
        [string]$SizeName,
        [int]$Width,
        [int]$Height,
        [switch]$ContractProbe
    )
    $arguments = if ($ContractProbe) { "--windows-contract-probe" } else { "--scene $Scene" }
    $name = if ($ContractProbe) {
        "contract-$Appearance-$SizeName"
    }
    else {
        "$Scene-$Appearance-$SizeName"
    }
    $start = [System.Diagnostics.ProcessStartInfo]::new()
    $start.FileName = (Resolve-Path $Binary).Path
    $start.Arguments = $arguments
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.EnvironmentVariables["RINKA_WINDOWS_APPEARANCE"] = $Appearance
    $process = [System.Diagnostics.Process]::Start($start)
    $caseResult = $null
    $probeFailure = $null
    $standardOutput = ""
    $standardError = ""
    try {
        $windows = Wait-ProcessWindows -Process $process
        Start-Sleep -Milliseconds 750
        $windows = [RinkaNativeProbe]::ProcessWindows([uint32]$process.Id)
        $expectedTitle = if ($ContractProbe) { "Rinka Windows Native Contract" } else { "Rinka Explorer" }
        $handle = [IntPtr]::Zero
        foreach ($candidate in $windows) {
            $candidateRoot = [System.Windows.Automation.AutomationElement]::FromHandle($candidate)
            $candidateClass = [RinkaNativeProbe]::ClassName($candidate)
            $isExpectedWindow = if ($ContractProbe) {
                $candidateRoot.Current.Name -eq $expectedTitle -and
                    $candidateClass -eq "Rinka.Window.Server2025"
            }
            else {
                $candidateClass -eq "WinUIDesktopWin32WindowClass" -and
                    $candidateRoot.Current.Name -in @("Rinka Explorer", "Remote Project")
            }
            if ($isExpectedWindow) {
                $handle = $candidate
                break
            }
        }
        if ($handle -eq [IntPtr]::Zero) {
            $observedWindows = @($windows | ForEach-Object {
                $root = [System.Windows.Automation.AutomationElement]::FromHandle($_)
                "'$($root.Current.Name)' ($([RinkaNativeProbe]::ClassName($_)))"
            })
            throw "Expected top-level window '$expectedTitle' was not found; observed: $($observedWindows -join ', ')"
        }
        $nativeHeightAdjustment = if ($ContractProbe) { 0 } else { 18 }
        $content = Set-ContentRectangle `
            -Handle $handle `
            -Width $Width `
            -Height $Height `
            -NativeHeightAdjustment $nativeHeightAdjustment
        $null = [RinkaNativeProbe]::SetForegroundWindow($handle)
        Start-Sleep -Milliseconds 500
        $automation = Get-AutomationRecord -Handle $handle
        $automationPath = Join-Path $OutputDirectory "$name.automation.json"
        $automation | ConvertTo-Json -Depth 6 | Set-Content -Path $automationPath -Encoding utf8
        $accessibility = Get-AccessibilityRecord -Handle $handle
        $accessibilityPath = Join-Path $OutputDirectory "$name.accessibility.json"
        $accessibility | ConvertTo-Json -Depth 6 | Set-Content -Path $accessibilityPath -Encoding utf8
        $requiredNames = if ($ContractProbe) {
            @(
                "Navigation pane",
                "Details pane",
                "Contract navigation",
                "Apply contract values",
                "Filter native controls",
                "Include hidden items",
                "Contract progress 62 percent",
                "Native controls are active"
            )
        }
        else {
            $sceneNames = switch ($Scene) {
                "ready" {
                    @(
                        "Files in Remote Project",
                        "Show hidden files"
                    )
                }
                "empty" { @("This folder is empty") }
                "busy" { @("Refreshing Remote Project", "Directory refresh 58 percent") }
                "error" { @("Remote Project is unavailable", "Reconnect to Remote Project") }
            }
            if ($Scene -eq "ready" -and $SizeName -eq "wide") {
                $sceneNames += "Open Cargo.toml in editor"
            }
            $workspaceNames = @(
                "Toggle Navigation",
                "Search files",
                "Locations"
            )
            if ($SizeName -eq "wide") {
                $workspaceNames += "Inspector"
            }
            $workspaceNames + $sceneNames
        }
        $semanticRecords = if ($ContractProbe) { $accessibility } else { $automation }
        try {
            Require-AccessibleNames `
                -Records $semanticRecords `
                -Names $requiredNames `
                -CaseName $expectedTitle
        }
        catch {
            Save-WindowCapture `
                -Handle $handle `
                -Path (Join-Path $OutputDirectory "$name.failed.png")
            throw
        }
        if (-not $ContractProbe) {
            $forward = @($semanticRecords | Where-Object { $_.name -eq "Forward" })
            if ($forward.Count -ne 1 -or $forward[0].enabled) {
                throw "Forward must be exposed once and disabled"
            }
        }
        $classes = @($automation | ForEach-Object { $_.class_name } | Sort-Object -Unique)
        $rootClass = [RinkaNativeProbe]::ClassName($handle)
        $expectedRootClass = if ($ContractProbe) {
            "Rinka.Window.Server2025"
        }
        else {
            "WinUIDesktopWin32WindowClass"
        }
        if ($rootClass -ne $expectedRootClass) {
            throw "Unexpected native root class '$rootClass'"
        }
        $context = [RinkaNativeProbe]::GetWindowDpiAwarenessContext($handle)
        $perMonitorV2 = [IntPtr](-4)
        if (-not [RinkaNativeProbe]::AreDpiAwarenessContextsEqual($context, $perMonitorV2)) {
            throw "Window is not running in PerMonitorV2 DPI awareness"
        }
        $capturePath = Join-Path $OutputDirectory "$name.png"
        $panelCaptures = [System.Collections.Generic.List[string]]::new()
        $panelCaptureLuminances = [System.Collections.Generic.List[double]]::new()
        $panelOwners = [System.Collections.Generic.List[long]]::new()
        $panelClients = [System.Collections.Generic.List[object]]::new()
        $panelContents = [System.Collections.Generic.List[object]]::new()
        $panelAutomation = [System.Collections.Generic.List[object]]::new()
        $panelAccessibility = [System.Collections.Generic.List[object]]::new()
        $panelClosePreservedMain = $null
        if ($Scene -eq "busy") {
            $panelHandles = [System.Collections.Generic.List[IntPtr]]::new()
            $panelIndex = 0
            foreach ($candidate in $windows) {
                if ($candidate -eq $handle) {
                    continue
                }
                $candidateRoot = [System.Windows.Automation.AutomationElement]::FromHandle($candidate)
                if ($candidateRoot.Current.Name -ne "Connection Activity") {
                    throw "Unexpected Busy-scene top-level window '$($candidateRoot.Current.Name)'"
                }
                $panelOwner = [RinkaNativeProbe]::GetWindow($candidate, 4)
                if ($panelOwner -ne $handle) {
                    throw "Connection Activity is not a native owned window of the main window"
                }
                $panelDpi = [RinkaNativeProbe]::GetDpiForWindow($candidate)
                $panelClient = Get-ClientRectangle -Handle $candidate
                $declaredPanelWidth = [int][Math]::Round(380 * $panelDpi / 96.0)
                $declaredPanelHeight = [int][Math]::Round(160 * $panelDpi / 96.0)
                $expectedPanelClientWidth = [int][Math]::Round(344 * $panelDpi / 96.0)
                $expectedPanelClientHeight = [int][Math]::Round(217 * $panelDpi / 96.0)
                if ($panelClient.width -ne $expectedPanelClientWidth -or
                    $panelClient.height -ne $expectedPanelClientHeight) {
                    throw "Connection Activity CompactOverlay client is $($panelClient.width)x$($panelClient.height), expected ${expectedPanelClientWidth}x${expectedPanelClientHeight}"
                }
                $panelContent = [ordered]@{
                    declared_width = $declaredPanelWidth
                    declared_height = $declaredPanelHeight
                    native_width = $panelClient.width
                    native_height = $panelClient.height
                    presenter = "CompactOverlay"
                }
                $panelFrame = Get-WindowRectangle -Handle $candidate
                $panelElements = @(Get-AutomationRecord -Handle $candidate)
                $panelAccessibleElements = @(Get-AccessibilityRecord -Handle $candidate)
                Assert-AutomationWithinFrame `
                    -Automation $panelElements `
                    -Frame $panelFrame `
                    -WindowName "Connection Activity"
                $panelNames = @($panelElements | ForEach-Object { $_.name })
                foreach ($requiredName in @(
                    "Refreshing Remote Project",
                    "Directory refresh 58 percent",
                    "Reading directory metadata",
                    "Stop directory refresh"
                )) {
                    if ($panelNames -notcontains $requiredName) {
                        throw "Connection Activity is missing accessible name '$requiredName'"
                    }
                }
                $panelPath = Join-Path $OutputDirectory "$name-panel-$panelIndex.png"
                Save-WindowCapture -Handle $candidate -Path $panelPath
                $panelLuminance = Get-CaptureLuminance -Path $panelPath
                if ($Appearance -eq "light" -and $panelLuminance -lt 160) {
                    throw "$name panel rendered dark pixels for the requested light appearance (luminance $panelLuminance)"
                }
                if ($Appearance -eq "dark" -and $panelLuminance -gt 140) {
                    throw "$name panel rendered light pixels for the requested dark appearance (luminance $panelLuminance)"
                }
                $panelCaptures.Add($panelPath)
                $panelCaptureLuminances.Add($panelLuminance)
                $panelOwners.Add($panelOwner.ToInt64())
                $panelClients.Add($panelClient)
                $panelContents.Add($panelContent)
                $panelAutomation.Add($panelElements)
                $panelAccessibility.Add($panelAccessibleElements)
                $panelHandles.Add($candidate)
                $panelIndex += 1
            }
            if ($panelCaptures.Count -ne 1) {
                throw "Busy scene must expose exactly one native activity panel"
            }
            Close-PanelAndRequireMainWindow `
                -Process $process `
                -PanelHandle $panelHandles[0] `
                -MainHandle $handle
            $panelClosePreservedMain = $true
        }
        Save-WindowCapture -Handle $handle -Path $capturePath
        $captureLuminance = Get-CaptureLuminance -Path $capturePath
        if ($Appearance -eq "light" -and $captureLuminance -lt 160) {
            throw "$name rendered dark pixels for the requested light appearance (luminance $captureLuminance)"
        }
        if ($Appearance -eq "dark" -and $captureLuminance -gt 140) {
            throw "$name rendered light pixels for the requested dark appearance (luminance $captureLuminance)"
        }
        $cycles = $null
        if ($Scene -eq "ready" -and $Appearance -eq "light" -and $SizeName -eq "wide") {
            $cycles = [ordered]@{
                navigation = Invoke-AutomationPaneCycles -Handle $handle -ButtonName "Toggle Navigation"
                details = Invoke-AutomationPaneCycles -Handle $handle -ButtonName "Details"
            }
        }
        $caseResult = [ordered]@{
            name = $name
            scene = $Scene
            appearance = $Appearance
            size = $SizeName
            process_id = $process.Id
            root_hwnd = $handle.ToInt64()
            root_class = $rootClass
            dpi = [RinkaNativeProbe]::GetDpiForWindow($handle)
            dpi_awareness = "PerMonitorV2"
            frame = Get-WindowRectangle -Handle $handle
            client = Get-ClientRectangle -Handle $handle
            content = $content
            top_level_window_count = $windows.Count
            native_classes = $classes
            automation = $automation
            automation_evidence = $automationPath
            accessibility = $accessibility
            accessibility_evidence = $accessibilityPath
            pane_cycles = $cycles
            capture = $capturePath
            capture_luminance = $captureLuminance
            panel_captures = $panelCaptures
            panel_capture_luminances = $panelCaptureLuminances
            panel_owner_hwnds = $panelOwners
            panel_clients = $panelClients
            panel_contents = $panelContents
            panel_automation = $panelAutomation
            panel_accessibility = $panelAccessibility
            panel_close_preserved_main = $panelClosePreservedMain
        }
    }
    catch {
        $probeFailure = $_
    }
    finally {
        Stop-ProbeProcess -Process $process
        $process.Refresh()
        $standardOutput = $process.StandardOutput.ReadToEnd()
        $standardError = $process.StandardError.ReadToEnd()
    }
    $stdoutPath = Join-Path $OutputDirectory "$name.stdout.txt"
    $stderrPath = Join-Path $OutputDirectory "$name.stderr.txt"
    $standardOutput | Set-Content -Path $stdoutPath -Encoding utf8
    $standardError | Set-Content -Path $stderrPath -Encoding utf8
    if ($null -ne $probeFailure) {
        throw $probeFailure
    }
    if ($process.ExitCode -ne 0) {
        throw "$name exited with native process code $($process.ExitCode); see $stderrPath"
    }
    if (-not [string]::IsNullOrWhiteSpace($standardError)) {
        throw "$name emitted unexpected standard error; see $stderrPath"
    }
    $caseResult["exit_code"] = $process.ExitCode
    $caseResult["stdout"] = $stdoutPath
    $caseResult["stderr"] = $stderrPath
    return $caseResult
}

$resolvedBinary = Resolve-Path $Binary
if (-not (Test-Path $resolvedBinary -PathType Leaf)) {
    throw "Explorer binary does not exist at '$Binary'"
}
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null

$results = [System.Collections.Generic.List[object]]::new()
$results.Add((Start-ProbeCase -Scene "contract" -Appearance "light" -SizeName "wide" -Width 1120 -Height 720 -ContractProbe))

foreach ($scene in @("ready", "empty", "busy", "error")) {
    foreach ($appearance in @("light", "dark")) {
        $results.Add((Start-ProbeCase -Scene $scene -Appearance $appearance -SizeName "wide" -Width 1120 -Height 720))
        $results.Add((Start-ProbeCase -Scene $scene -Appearance $appearance -SizeName "narrow" -Width 760 -Height 520))
    }
}

$requiredFallbackClasses = @(
    "Button",
    "Edit",
    "msctls_progress32",
    "Rinka.Window.Server2025",
    "Static",
    "SysTreeView32"
)
$requiredWinUiClasses = @(
    "AppBarButton",
    "AutoSuggestBox",
    "ListView",
    "Microsoft.UI.Xaml.Controls.NavigationViewItem",
    "Microsoft.UI.Xaml.Controls.ProgressBar",
    "Microsoft.UI.Xaml.Controls.TitleBar",
    "TextBox",
    "ToggleSwitch",
    "WinUIDesktopWin32WindowClass"
)
$requiredClasses = $requiredFallbackClasses + $requiredWinUiClasses
$observedClasses = @($results | ForEach-Object { $_.native_classes } | Sort-Object -Unique)
foreach ($requiredClass in $requiredClasses) {
    if ($observedClasses -notcontains $requiredClass) {
        throw "Required native class '$requiredClass' was not observed"
    }
}

$result = [ordered]@{
    schema = 1
    platform = "Windows Server 2025 Desktop Experience"
    captured_at_utc = [DateTime]::UtcNow.ToString("o")
    binary = $resolvedBinary.Path
    cases = $results
    required_fallback_classes = $requiredFallbackClasses
    required_winui_classes = $requiredWinUiClasses
    observed_native_classes = $observedClasses
    image_count = @(
        $results | ForEach-Object {
            $_.capture
            $_.panel_captures
        }
    ).Count
    pane_cycle_count = 3
    result = "PASS"
}
$jsonPath = Join-Path $OutputDirectory "windows-scene-matrix.json"
$result | ConvertTo-Json -Depth 12 | Set-Content -Path $jsonPath -Encoding utf8
$result | ConvertTo-Json -Depth 5
