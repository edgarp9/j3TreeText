param(
    [switch]$NoBuild,
    [switch]$KeepTemp
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$commonPath = Join-Path $repoRoot "src\platform\win32\common.rs"
$constants = @{}

foreach ($line in Get-Content -LiteralPath $commonPath) {
    if ($line -match 'pub\(super\) const (COMMAND_[A-Z0-9_]+|CONTROL_[A-Z0-9_]+): usize = ([0-9]+);') {
        $constants[$matches[1]] = [int]$matches[2]
    } elseif ($line -match 'pub\(super\) const WINDOW_CLASS_NAME: &str = "([^"]+)";') {
        $windowClassName = $matches[1]
    }
}

if (-not $windowClassName) {
    throw "WINDOW_CLASS_NAME was not found in $commonPath"
}

$requiredConstants = @(
    "COMMAND_SAVE_DOCUMENT",
    "COMMAND_IMPORT_TEXT",
    "COMMAND_EXPORT_TEXT",
    "COMMAND_EXPORT_ALL_TEXT",
    "COMMAND_CLOSE_TAB",
    "COMMAND_CLOSE_WINDOW",
    "COMMAND_EDITOR_UNDO",
    "COMMAND_EDITOR_CUT",
    "COMMAND_EDITOR_COPY",
    "COMMAND_EDITOR_PASTE",
    "COMMAND_EDITOR_DELETE_SELECTION",
    "COMMAND_EDITOR_SELECT_ALL",
    "COMMAND_FIND_TEXT",
    "COMMAND_REPLACE_TEXT",
    "COMMAND_NEW_DOCUMENT",
    "COMMAND_NEW_CHILD_DOCUMENT",
    "COMMAND_RENAME",
    "COMMAND_MOVE_UP",
    "COMMAND_MOVE_DOWN",
    "COMMAND_DELETE",
    "COMMAND_SHOW_ACTIVE_TREE",
    "COMMAND_SHOW_TRASH",
    "COMMAND_RESTORE",
    "COMMAND_DELETE_PERMANENTLY",
    "COMMAND_EDITOR_WORD_WRAP",
    "COMMAND_EDITOR_FONT",
    "COMMAND_ABOUT",
    "COMMAND_IMPORT_ENCODING_UTF8",
    "COMMAND_IMPORT_ENCODING_UTF8_BOM",
    "COMMAND_IMPORT_ENCODING_UTF16_LE_BOM",
    "COMMAND_IMPORT_ENCODING_UTF16_BE_BOM",
    "COMMAND_IMPORT_ENCODING_KOREAN_EUC_KR",
    "COMMAND_IMPORT_ENCODING_WINDOWS_1252",
    "COMMAND_IMPORT_ENCODING_AUTO",
    "COMMAND_EXPORT_ENCODING_UTF8_BOM",
    "COMMAND_EXPORT_ENCODING_UTF8",
    "COMMAND_EXPORT_ENCODING_UTF16_LE_BOM",
    "COMMAND_EXPORT_ENCODING_UTF16_BE_BOM",
    "COMMAND_EXPORT_ENCODING_KOREAN_EUC_KR",
    "COMMAND_EXPORT_ENCODING_WINDOWS_1252",
    "COMMAND_THEME_FOREST",
    "COMMAND_THEME_LIGHT",
    "COMMAND_THEME_CLASSIC_DARK",
    "COMMAND_THEME_SEPIA_TEAL",
    "COMMAND_THEME_GRAPHITE",
    "COMMAND_THEME_STEEL_BLUE",
    "COMMAND_LANGUAGE_KOREAN",
    "COMMAND_LANGUAGE_ENGLISH",
    "CONTROL_EDITOR_ID",
    "CONTROL_TREE_ID",
    "CONTROL_SEARCH_ID",
    "CONTROL_TAB_ID",
    "CONTROL_CARET_STATUS_ID"
)

foreach ($name in $requiredConstants) {
    if (-not $constants.ContainsKey($name)) {
        throw "$name was not found in $commonPath"
    }
}

$declaredCommandConstants = @($constants.Keys | Where-Object { $_ -like "COMMAND_*" } | Sort-Object)
$missingRequiredCommandConstants = @($declaredCommandConstants | Where-Object { $requiredConstants -notcontains $_ })
if ($missingRequiredCommandConstants.Count -gt 0) {
    throw "Win32 smoke requiredConstants is missing command constants: $($missingRequiredCommandConstants -join ', ')"
}

$scriptPath = if ($PSCommandPath) { $PSCommandPath } else { $MyInvocation.MyCommand.Path }
$scriptSource = Get-Content -LiteralPath $scriptPath -Raw
$runtimeStart = $scriptSource.IndexOf('$process = Start-Process')
if ($runtimeStart -lt 0) {
    throw "Cannot locate Windows smoke runtime section"
}
$runtimeSmokeSource = $scriptSource.Substring($runtimeStart)
$missingRuntimeCommandReferences = @(
    $declaredCommandConstants | Where-Object {
        $runtimeSmokeSource -notmatch [regex]::Escape("`"$_`"")
    }
)
if ($missingRuntimeCommandReferences.Count -gt 0) {
    throw "Win32 smoke runtime does not exercise command constants: $($missingRuntimeCommandReferences -join ', ')"
}

if (-not $NoBuild) {
    Push-Location $repoRoot
    try {
        & cargo build --bins
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build --bins failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

$nativeSource = @"
using System;
using System.Runtime.InteropServices;
using System.Text;

public struct J3Rect {
    public int Left;
    public int Top;
    public int Right;
    public int Bottom;
}

public static class J3Native {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool EnumChildWindows(IntPtr hWndParent, EnumWindowsProc lpEnumFunc, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);

    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern int GetClassName(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);

    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern int GetWindowTextLength(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool IsWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern IntPtr GetDlgItem(IntPtr hDlg, int nIDDlgItem);

    [DllImport("user32.dll")]
    public static extern int GetDlgCtrlID(IntPtr hWnd);

    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern bool SetWindowText(IntPtr hWnd, string lpString);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool GetWindowRect(IntPtr hWnd, out J3Rect lpRect);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool MoveWindow(IntPtr hWnd, int X, int Y, int nWidth, int nHeight, bool bRepaint);

    [DllImport("user32.dll")]
    public static extern IntPtr SetFocus(IntPtr hWnd);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr SendMessage(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessage(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern IntPtr GetMenu(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern IntPtr GetSubMenu(IntPtr hMenu, int nPos);

    [DllImport("user32.dll")]
    public static extern int GetMenuItemCount(IntPtr hMenu);

    [DllImport("user32.dll")]
    public static extern uint GetMenuItemID(IntPtr hMenu, int nPos);

    [DllImport("user32.dll")]
    public static extern bool GetMenuItemRect(IntPtr hWnd, IntPtr hMenu, uint uItem, out J3Rect lprcItem);

    [DllImport("user32.dll")]
    public static extern int GetMenuState(IntPtr hMenu, uint uId, uint uFlags);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetMenuString(IntPtr hMenu, uint uIDItem, StringBuilder lpString, int cchMax, uint flags);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);

    [DllImport("user32.dll")]
    public static extern bool SetCursorPos(int X, int Y);

    [DllImport("user32.dll")]
    public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, UIntPtr dwExtraInfo);

    [DllImport("user32.dll")]
    public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
}
"@

if (-not ("J3Native" -as [type])) {
    Add-Type -TypeDefinition $nativeSource
}

$WM_COMMAND = 0x0111
$WM_CLOSE = 0x0010
$WM_KEYDOWN = 0x0100
$WM_CHAR = 0x0102
$WM_SETTEXT = 0x000C
$BM_CLICK = 0x00F5
$MF_BYCOMMAND = 0x00000000
$MF_BYPOSITION = 0x00000400
$MF_CHECKED = 0x00000008
$MF_GRAYED = 0x00000001
$MF_DISABLED = 0x00000002
$IDOK = 1
$IDCANCEL = 2
$IDYES = 6
$KEYEVENTF_KEYUP = 0x0002
$MOUSEEVENTF_LEFTDOWN = 0x0002
$MOUSEEVENTF_LEFTUP = 0x0004
$SW_RESTORE = 9
$VK_CONTROL_KEY = 0x11
$VK_A_KEY = 0x41
$VK_F_KEY = 0x46
$VK_H_KEY = 0x48
$VK_N_KEY = 0x4E
$VK_S_KEY = 0x53
$VK_W_KEY = 0x57
$VK_DELETE_KEY = 0x2E
$VK_F2_KEY = 0x71
$VK_RETURN = 0x0D

$TV_FIRST = 0x1100
$TVM_GETEDITCONTROL = $TV_FIRST + 15
$TVM_ENDEDITLABELNOW = $TV_FIRST + 22

function Assert-True($condition, $message) {
    if (-not $condition) {
        throw $message
    }
}

function Wait-Until($description, [scriptblock]$predicate, [int]$timeoutMs = 8000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($timeoutMs)
    do {
        $value = & $predicate
        $ready = $false
        if ($null -ne $value) {
            if ($value -is [IntPtr]) {
                $ready = $value -ne [IntPtr]::Zero
            } elseif ($value -is [bool]) {
                $ready = $value
            } else {
                $ready = [bool]$value
            }
        }
        if ($ready) {
            return $value
        }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "Timed out waiting for $description"
}

function Get-WindowClass([IntPtr]$hwnd) {
    $builder = [Text.StringBuilder]::new(256)
    [void][J3Native]::GetClassName($hwnd, $builder, $builder.Capacity)
    $builder.ToString()
}

function Get-WindowTextValue([IntPtr]$hwnd) {
    $length = [J3Native]::GetWindowTextLength($hwnd)
    $builder = [Text.StringBuilder]::new([Math]::Max($length + 1, 256))
    [void][J3Native]::GetWindowText($hwnd, $builder, $builder.Capacity)
    $builder.ToString()
}

function Get-WindowRectValue([IntPtr]$hwnd) {
    $rect = [J3Rect]::new()
    Assert-True ([J3Native]::GetWindowRect($hwnd, [ref]$rect)) "GetWindowRect failed for $hwnd"
    [pscustomobject]@{
        Left = $rect.Left
        Top = $rect.Top
        Right = $rect.Right
        Bottom = $rect.Bottom
        Width = $rect.Right - $rect.Left
        Height = $rect.Bottom - $rect.Top
    }
}

function Get-ProcessTopWindows([int]$processId) {
    $windows = New-Object System.Collections.Generic.List[object]
    $callback = [J3Native+EnumWindowsProc]{
        param([IntPtr]$hwnd, [IntPtr]$lparam)
        [uint32]$ownerPid = 0
        [void][J3Native]::GetWindowThreadProcessId($hwnd, [ref]$ownerPid)
        if ($ownerPid -eq $processId) {
            $windows.Add([pscustomobject]@{
                Handle = $hwnd
                Class = Get-WindowClass $hwnd
                Title = Get-WindowTextValue $hwnd
                Visible = [J3Native]::IsWindowVisible($hwnd)
            })
        }
        return $true
    }
    [void][J3Native]::EnumWindows($callback, [IntPtr]::Zero)
    $windows
}

function Get-ChildWindows([IntPtr]$parent) {
    $children = New-Object System.Collections.Generic.List[object]
    $callback = [J3Native+EnumWindowsProc]{
        param([IntPtr]$hwnd, [IntPtr]$lparam)
        $children.Add([pscustomobject]@{
            Handle = $hwnd
            Class = Get-WindowClass $hwnd
            Title = Get-WindowTextValue $hwnd
            Visible = [J3Native]::IsWindowVisible($hwnd)
            Id = [J3Native]::GetDlgCtrlID($hwnd)
        })
        return $true
    }
    [void][J3Native]::EnumChildWindows($parent, $callback, [IntPtr]::Zero)
    $children
}

function Describe-WindowTree([int]$processId) {
    $lines = New-Object System.Collections.Generic.List[string]
    foreach ($top in Get-ProcessTopWindows $processId) {
        $lines.Add("top handle=$($top.Handle) class=$($top.Class) visible=$($top.Visible) title=$($top.Title)")
        foreach ($child in Get-ChildWindows $top.Handle) {
            $lines.Add("  child handle=$($child.Handle) id=$($child.Id) class=$($child.Class) visible=$($child.Visible) title=$($child.Title)")
        }
    }
    $lines -join "`n"
}

function Find-AppWindow($processId) {
    $windowMatches = @(Get-ProcessTopWindows $processId |
        Where-Object { $_.Class -eq $windowClassName -and $_.Visible })
    if ($windowMatches.Count -gt 0) {
        return $windowMatches[0].Handle
    }
    return [IntPtr]::Zero
}

function Resolve-AppWindow([IntPtr]$candidate) {
    if ($candidate -ne [IntPtr]::Zero -and (Get-WindowClass $candidate) -eq $windowClassName) {
        return $candidate
    }
    if ($script:SmokeProcessId) {
        $resolved = Find-AppWindow $script:SmokeProcessId
        if ($resolved -ne [IntPtr]::Zero) {
            return $resolved
        }
    }
    return $candidate
}

function Find-Dialog($processId, [string]$titlePattern = $null) {
    $dialogs = Get-ProcessTopWindows $processId |
        Where-Object { $_.Class -eq "#32770" -and $_.Visible }
    if ($titlePattern) {
        $dialogs = $dialogs | Where-Object { $_.Title -match $titlePattern }
    }
    $first = $dialogs | Select-Object -First 1
    if ($first) {
        return $first.Handle
    }
    return [IntPtr]::Zero
}

function Wait-Dialog($processId, [string]$titlePattern = $null, [int]$timeoutMs = 8000) {
    Wait-Until "dialog '$titlePattern'" { Find-Dialog $processId $titlePattern } $timeoutMs
}

function Close-Dialog([IntPtr]$dialog, [int]$buttonId = $IDCANCEL) {
    Assert-True ([J3Native]::IsWindow($dialog)) "Dialog handle is stale before close"
    $button = [J3Native]::GetDlgItem($dialog, $buttonId)
    if ($button -ne [IntPtr]::Zero) {
        [void][J3Native]::SendMessage($button, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
    } else {
        [void][J3Native]::PostMessage($dialog, $WM_COMMAND, [IntPtr]$buttonId, [IntPtr]::Zero)
    }
    try {
        Wait-Until "dialog to close" { -not [J3Native]::IsWindow($dialog) } 5000 | Out-Null
    } catch {
        if ($buttonId -eq $IDOK -or $buttonId -eq $IDCANCEL) {
            [void][J3Native]::PostMessage($dialog, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
            Wait-Until "dialog to close after WM_CLOSE" { -not [J3Native]::IsWindow($dialog) } 5000 | Out-Null
        } else {
            throw
        }
    }
}

function Send-Command([IntPtr]$hwnd, [string]$name) {
    $hwnd = Resolve-AppWindow $hwnd
    $commandId = [IntPtr]([int]$constants[$name])
    [void][J3Native]::SendMessage($hwnd, $WM_COMMAND, $commandId, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 120
}

function Post-Command([IntPtr]$hwnd, [string]$name) {
    $hwnd = Resolve-AppWindow $hwnd
    $commandId = [IntPtr]([int]$constants[$name])
    Assert-True ([J3Native]::PostMessage($hwnd, $WM_COMMAND, $commandId, [IntPtr]::Zero)) "PostMessage failed for $name"
}

function Get-Control([IntPtr]$hwnd, [string]$name) {
    try {
        Wait-Until "control $name" {
            $control = [J3Native]::GetDlgItem($hwnd, $constants[$name])
            if ($control -ne [IntPtr]::Zero) { $control } else { $null }
        } 8000
    } catch {
        $targetId = [int]$constants[$name]
        if ($script:SmokeProcessId) {
            foreach ($top in Get-ProcessTopWindows $script:SmokeProcessId) {
                foreach ($child in Get-ChildWindows $top.Handle) {
                    if ([int]$child.Id -eq $targetId -and [bool]$child.Visible) {
                        return $child.Handle
                    }
                }
            }
        } else {
            foreach ($child in Get-ChildWindows $hwnd) {
                if ([int]$child.Id -eq $targetId -and [bool]$child.Visible) {
                    return $child.Handle
                }
            }
        }
        $fallbackClass = switch ($name) {
            "CONTROL_EDITOR_ID" { "RICHEDIT50W" }
            "CONTROL_TREE_ID" { "SysTreeView32" }
            "CONTROL_SEARCH_ID" { "Edit" }
            "CONTROL_TAB_ID" { "SysTabControl32" }
            default { $null }
        }
        if ($fallbackClass) {
            $fallback = if ($script:SmokeProcessId) {
                foreach ($top in Get-ProcessTopWindows $script:SmokeProcessId) {
                    Get-ChildWindows $top.Handle |
                        Where-Object { $_.Class -eq $fallbackClass -and $_.Visible } |
                        Select-Object -First 1
                }
            } else {
                Get-ChildWindows $hwnd |
                    Where-Object { $_.Class -eq $fallbackClass -and $_.Visible } |
                    Select-Object -First 1
            }
            if ($fallback) {
                return ($fallback | Select-Object -First 1).Handle
            }
        }
        $diagnostic = if ($script:SmokeProcessId) {
            Describe-WindowTree $script:SmokeProcessId
        } else {
            "process id was not set"
        }
        throw "$($_.Exception.Message)`nWindow tree:`n$diagnostic"
    }
}

function Set-ControlText([IntPtr]$hwnd, [string]$text) {
    Assert-True ([J3Native]::SetWindowText($hwnd, $text)) "SetWindowText failed"
    Start-Sleep -Milliseconds 120
}

function Type-ControlText([IntPtr]$hwnd, [string]$text) {
    [void][J3Native]::SetFocus($hwnd)
    foreach ($ch in $text.ToCharArray()) {
        [void][J3Native]::SendMessage($hwnd, $WM_CHAR, [IntPtr]([int][char]$ch), [IntPtr]1)
    }
    Start-Sleep -Milliseconds 120
}

function Activate-AppWindow([IntPtr]$hwnd) {
    $hwnd = Resolve-AppWindow $hwnd
    [void][J3Native]::ShowWindow($hwnd, $SW_RESTORE)
    [void][J3Native]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 120
}

function Press-KeyChord([IntPtr]$hwnd, [IntPtr]$focus, [int[]]$keys) {
    Activate-AppWindow $hwnd
    [void][J3Native]::SetFocus($focus)
    Start-Sleep -Milliseconds 80
    $lastIndex = $keys.Length - 1
    for ($index = 0; $index -lt $lastIndex; $index++) {
        [J3Native]::keybd_event([byte]$keys[$index], 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 30
    }
    Assert-True ([J3Native]::PostMessage($focus, $WM_KEYDOWN, [IntPtr]$keys[$lastIndex], [IntPtr]1)) "PostMessage failed for shortcut key $($keys[$lastIndex])"
    Start-Sleep -Milliseconds 250
    for ($index = $lastIndex - 1; $index -ge 0; $index--) {
        [J3Native]::keybd_event([byte]$keys[$index], 0, [uint32]$KEYEVENTF_KEYUP, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 30
    }
    Start-Sleep -Milliseconds 250
}

function End-TreeLabelEdit([IntPtr]$tree, [string]$title = $null, [switch]$Cancel) {
    $edit = Wait-Until "tree label edit" {
        $handle = [J3Native]::SendMessage($tree, $TVM_GETEDITCONTROL, [IntPtr]::Zero, [IntPtr]::Zero)
        if ($handle -ne [IntPtr]::Zero) { $handle } else { $null }
    } 5000
    if ($title) {
        Type-ControlText $edit $title
    }
    $cancelValue = if ($Cancel) { 1 } else { 0 }
    [void][J3Native]::SendMessage($tree, $TVM_ENDEDITLABELNOW, [IntPtr]$cancelValue, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 250
}

function Get-MenuItemState([IntPtr]$hwnd, [string]$name) {
    $hwnd = Resolve-AppWindow $hwnd
    $menu = [J3Native]::GetMenu($hwnd)
    Assert-True ($menu -ne [IntPtr]::Zero) "Main menu was not attached"
    $commandId = [uint32]([int]$constants[$name])
    [J3Native]::GetMenuState($menu, $commandId, [uint32]$MF_BYCOMMAND)
}

function Move-ScreenPoint([int]$x, [int]$y) {
    Assert-True ([J3Native]::SetCursorPos($x, $y)) "SetCursorPos failed"
    Start-Sleep -Milliseconds 80
}

function Click-ScreenPoint([int]$x, [int]$y) {
    Move-ScreenPoint $x $y
    [J3Native]::mouse_event([uint32]$MOUSEEVENTF_LEFTDOWN, [uint32]0, [uint32]0, [uint32]0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 40
    [J3Native]::mouse_event([uint32]$MOUSEEVENTF_LEFTUP, [uint32]0, [uint32]0, [uint32]0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 250
}

function Get-MenuItemRectValue([IntPtr]$hwnd, [IntPtr]$menu, [int]$index) {
    $rect = [J3Rect]::new()
    Assert-True ([J3Native]::GetMenuItemRect($hwnd, $menu, [uint32]$index, [ref]$rect)) "GetMenuItemRect failed for index $index"
    [pscustomobject]@{
        Left = $rect.Left
        Top = $rect.Top
        Right = $rect.Right
        Bottom = $rect.Bottom
        Width = $rect.Right - $rect.Left
        Height = $rect.Bottom - $rect.Top
        CenterX = [int](($rect.Left + $rect.Right) / 2)
        CenterY = [int](($rect.Top + $rect.Bottom) / 2)
    }
}

function Find-MenuPathByCommandId([IntPtr]$menu, [int]$commandId) {
    $count = [J3Native]::GetMenuItemCount($menu)
    for ($index = 0; $index -lt $count; $index++) {
        $id = [J3Native]::GetMenuItemID($menu, $index)
        if ($id -eq [uint32]$commandId) {
            return @($index)
        }
        $submenu = [J3Native]::GetSubMenu($menu, $index)
        if ($submenu -ne [IntPtr]::Zero) {
            $childPath = @(Find-MenuPathByCommandId $submenu $commandId)
            if ($childPath.Count -gt 0) {
                return @($index) + $childPath
            }
        }
    }
    @()
}

function Click-MenuCommand([IntPtr]$hwnd, [int]$topIndex, [string]$name) {
    $hwnd = Resolve-AppWindow $hwnd
    Activate-AppWindow $hwnd
    $mainMenu = [J3Native]::GetMenu($hwnd)
    Assert-True ($mainMenu -ne [IntPtr]::Zero) "Main menu was not attached"
    $currentMenu = [J3Native]::GetSubMenu($mainMenu, $topIndex)
    Assert-True ($currentMenu -ne [IntPtr]::Zero) "Top menu $topIndex was not found"
    $commandId = [int]$constants[$name]
    $path = @(Find-MenuPathByCommandId $currentMenu $commandId)
    Assert-True ($path.Count -gt 0) "Menu command $name was not found below top index $topIndex"

    $topRect = Get-MenuItemRectValue $hwnd $mainMenu $topIndex
    Click-ScreenPoint $topRect.CenterX $topRect.CenterY

    for ($depth = 0; $depth -lt $path.Count; $depth++) {
        $itemIndex = [int]$path[$depth]
        $itemRect = Get-MenuItemRectValue $hwnd $currentMenu $itemIndex
        if ($depth -lt $path.Count - 1) {
            Move-ScreenPoint $itemRect.CenterX $itemRect.CenterY
            Start-Sleep -Milliseconds 350
            $currentMenu = [J3Native]::GetSubMenu($currentMenu, $itemIndex)
            Assert-True ($currentMenu -ne [IntPtr]::Zero) "Nested submenu for $name was not found at depth $depth"
            Start-Sleep -Milliseconds 250
        } else {
            Click-ScreenPoint $itemRect.CenterX $itemRect.CenterY
        }
    }
    Start-Sleep -Milliseconds 250
}

function Assert-MenuChecked([IntPtr]$hwnd, [string]$name, [bool]$expected) {
    $actual = Test-MenuChecked $hwnd $name
    $state = Get-MenuItemState $hwnd $name
    Assert-True ($actual -eq $expected) "$name checked state expected $expected, got $actual ($state)"
}

function Assert-MenuEnabled([IntPtr]$hwnd, [string]$name, [bool]$expected) {
    $actual = Test-MenuEnabled $hwnd $name
    $state = Get-MenuItemState $hwnd $name
    Assert-True ($actual -eq $expected) "$name enabled state expected $expected, got $actual ($state)"
}

function Test-MenuChecked([IntPtr]$hwnd, [string]$name) {
    $state = Get-MenuItemState $hwnd $name
    (($state -band $MF_CHECKED) -ne 0)
}

function Click-MenuCommandAndAssertCheckedState([IntPtr]$hwnd, [int]$topIndex, [string]$name, [bool]$expected) {
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        Click-MenuCommand $hwnd $topIndex $name
        if ((Test-MenuChecked $hwnd $name) -eq $expected) {
            return
        }
        Start-Sleep -Milliseconds 200
    }
    Assert-MenuChecked $hwnd $name $expected
}

function Click-MenuCommandAndAssertChecked([IntPtr]$hwnd, [int]$topIndex, [string]$name) {
    Click-MenuCommandAndAssertCheckedState $hwnd $topIndex $name $true
}

function Test-MenuEnabled([IntPtr]$hwnd, [string]$name) {
    $state = Get-MenuItemState $hwnd $name
    (($state -band ($MF_GRAYED -bor $MF_DISABLED)) -eq 0)
}

function Click-MenuCommandAndWaitDialog([IntPtr]$hwnd, [int]$topIndex, [string]$name, [int]$processId, [string]$titlePattern = $null) {
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        Click-MenuCommand $hwnd $topIndex $name
        try {
            return (Wait-Dialog $processId $titlePattern 3000)
        } catch {
            if ($attempt -eq 3) {
                throw
            }
        }
    }
}

function Click-MenuCommandAndWaitProcessExit([IntPtr]$hwnd, [int]$topIndex, [string]$name, $process) {
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        Click-MenuCommand $hwnd $topIndex $name
        try {
            Wait-Until "process exit after $name" { $process.Refresh(); $process.HasExited } 3000 | Out-Null
            return
        } catch {
            if ($attempt -eq 3) {
                throw
            }
        }
    }
}

function Get-TopMenuLabels([IntPtr]$hwnd) {
    $hwnd = Resolve-AppWindow $hwnd
    $menu = [J3Native]::GetMenu($hwnd)
    $labels = @()
    for ($index = 0; $index -lt 5; $index++) {
        $builder = [Text.StringBuilder]::new(128)
        [void][J3Native]::GetMenuString($menu, [uint32]$index, $builder, $builder.Capacity, [uint32]$MF_BYPOSITION)
        $labels += $builder.ToString()
    }
    $labels
}

function Assert-TopMenuLabels([IntPtr]$hwnd, [string[]]$expected) {
    $actual = @(Get-TopMenuLabels $hwnd)
    Assert-True ($actual.Count -eq $expected.Count) "Top menu count expected $($expected.Count), got $($actual.Count): $($actual -join ', ')"
    for ($index = 0; $index -lt $expected.Count; $index++) {
        Assert-True ($actual[$index] -eq $expected[$index]) "Top menu label[$index] expected '$($expected[$index])', got '$($actual[$index])'"
    }
}

function Text-FromCodePoints([int[]]$codePoints) {
    -join ($codePoints | ForEach-Object { [char]$_ })
}

function Korean-TopMenuLabels {
    @(
        (Text-FromCodePoints @(0xD30C, 0xC77C)),
        (Text-FromCodePoints @(0xD3B8, 0xC9D1)),
        (Text-FromCodePoints @(0xBB38, 0xC11C)),
        (Text-FromCodePoints @(0xBCF4, 0xAE30)),
        (Text-FromCodePoints @(0xB3C4, 0xC6C0, 0xB9D0))
    )
}

function Assert-LayoutLooksUsable([IntPtr]$hwnd, [IntPtr]$search, [IntPtr]$tree, [IntPtr]$tab, [IntPtr]$editor, [IntPtr]$caretStatus) {
    $windowRect = Get-WindowRectValue $hwnd
    $searchRect = Get-WindowRectValue $search
    $treeRect = Get-WindowRectValue $tree
    $tabRect = Get-WindowRectValue $tab
    $editorRect = Get-WindowRectValue $editor
    $statusRect = Get-WindowRectValue $caretStatus

    foreach ($item in @(
        @{ Name = "window"; Rect = $windowRect },
        @{ Name = "search"; Rect = $searchRect },
        @{ Name = "tree"; Rect = $treeRect },
        @{ Name = "tab"; Rect = $tabRect },
        @{ Name = "editor"; Rect = $editorRect },
        @{ Name = "caret status"; Rect = $statusRect }
    )) {
        Assert-True ($item.Rect.Width -gt 0 -and $item.Rect.Height -gt 0) "$($item.Name) has invalid rect: $($item.Rect | Out-String)"
    }

    Assert-True ($searchRect.Bottom -le $treeRect.Top) "Search box should be above the tree"
    Assert-True ($treeRect.Right -le $editorRect.Left) "Tree should be left of the editor"
    Assert-True ($tabRect.Bottom -le $editorRect.Top) "Tab bar should be above the editor"
    Assert-True ($statusRect.Top -ge $tabRect.Bottom) "Caret status should stay in the editor pane"
    Assert-True ($editorRect.Width -ge 120 -and $editorRect.Height -ge 80) "Editor should remain usable after layout"
}

function Run-Cli([string[]]$arguments) {
    $output = & $script:cliExe @arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "CLI failed: $($arguments -join ' ')`n$output"
    }
    ($output -join "`n")
}

function Assert-CliContains([string[]]$arguments, [string]$needle) {
    $text = Run-Cli $arguments
    Assert-True ($text -like "*$needle*") "CLI output did not contain '$needle'. Output:`n$text"
    $text
}

function Assert-CliNotContains([string[]]$arguments, [string]$needle) {
    $text = Run-Cli $arguments
    Assert-True ($text -notlike "*$needle*") "CLI output unexpectedly contained '$needle'. Output:`n$text"
    $text
}

function Open-And-Cancel-Dialog([IntPtr]$hwnd, [int]$processId, [string]$command, [string]$titlePattern = $null) {
    Post-Command $hwnd $command
    $dialog = Wait-Dialog $processId $titlePattern
    Close-Dialog $dialog $IDCANCEL
}

$tempRoot = Join-Path $env:TEMP ("j3treetext-win-menu-smoke-" + (Get-Date -Format "yyyyMMdd-HHmmss"))
New-Item -ItemType Directory -Path $tempRoot | Out-Null
$guiSource = Join-Path $repoRoot "target\debug\j3TreeText.exe"
$script:cliExe = Join-Path $repoRoot "target\debug\j3TreeTextCli.exe"
$guiExe = Join-Path $tempRoot "j3TreeText.exe"
Copy-Item -LiteralPath $guiSource -Destination $guiExe
$dbPath = Join-Path $tempRoot "j3TreeText.db"
$stderrPath = Join-Path $tempRoot "gui-stderr.log"

$process = Start-Process -FilePath $guiExe -WorkingDirectory $tempRoot -RedirectStandardError $stderrPath -PassThru
$script:SmokeProcessId = $process.Id
$script:SmokeSucceeded = $false

try {
    $hwnd = Wait-Until "main window" { Find-AppWindow $process.Id } 10000
    $hwnd = Resolve-AppWindow $hwnd
    Assert-True ((Get-WindowClass $hwnd) -eq $windowClassName) "Resolved main hwnd has unexpected class: $(Get-WindowClass $hwnd), value=$hwnd, type=$($hwnd.GetType().FullName)"
    [void][J3Native]::SetForegroundWindow($hwnd)
    Wait-Until "database creation" { Test-Path -LiteralPath $dbPath } 5000 | Out-Null

    $editor = Get-Control $hwnd "CONTROL_EDITOR_ID"
    $tree = Get-Control $hwnd "CONTROL_TREE_ID"
    $search = Get-Control $hwnd "CONTROL_SEARCH_ID"
    $tab = Get-Control $hwnd "CONTROL_TAB_ID"
    $caretStatus = Get-Control $hwnd "CONTROL_CARET_STATUS_ID"

    Assert-TopMenuLabels $hwnd @("File", "Edit", "Document", "View", "Help")
    Assert-LayoutLooksUsable $hwnd $search $tree $tab $editor $caretStatus
    Assert-True ([J3Native]::MoveWindow($hwnd, 120, 120, 1000, 720, $true)) "Main window resize failed"
    Start-Sleep -Milliseconds 250
    Assert-LayoutLooksUsable $hwnd $search $tree $tab $editor $caretStatus

    $initialAboutDialog = Click-MenuCommandAndWaitDialog $hwnd 4 "COMMAND_ABOUT" $process.Id
    Close-Dialog $initialAboutDialog $IDOK

    Press-KeyChord $hwnd $editor @($VK_CONTROL_KEY, $VK_F_KEY)
    $shortcutFindDialog = Wait-Dialog $process.Id
    Close-Dialog $shortcutFindDialog $IDCANCEL
    Press-KeyChord $hwnd $editor @($VK_CONTROL_KEY, $VK_H_KEY)
    $shortcutReplaceDialog = Wait-Dialog $process.Id
    Close-Dialog $shortcutReplaceDialog $IDCANCEL

    Type-ControlText $editor "windows menu smoke text"
    Click-MenuCommand $hwnd 0 "COMMAND_SAVE_DOCUMENT"
    Assert-CliContains @("--db", $dbPath, "show", "1") "windows menu smoke text" | Out-Null
    Press-KeyChord $hwnd $editor @($VK_CONTROL_KEY, $VK_S_KEY)
    Click-MenuCommand $hwnd 1 "COMMAND_EDITOR_SELECT_ALL"
    Click-MenuCommand $hwnd 1 "COMMAND_EDITOR_COPY"
    Click-MenuCommand $hwnd 1 "COMMAND_EDITOR_DELETE_SELECTION"
    Click-MenuCommand $hwnd 0 "COMMAND_SAVE_DOCUMENT"
    Assert-CliNotContains @("--db", $dbPath, "show", "1") "windows menu smoke text" | Out-Null
    Click-MenuCommand $hwnd 1 "COMMAND_EDITOR_PASTE"
    Click-MenuCommand $hwnd 0 "COMMAND_SAVE_DOCUMENT"
    Assert-CliContains @("--db", $dbPath, "show", "1") "windows menu smoke text" | Out-Null
    Send-Command $hwnd "COMMAND_EDITOR_SELECT_ALL"
    Click-MenuCommand $hwnd 1 "COMMAND_EDITOR_COPY"
    Click-MenuCommand $hwnd 1 "COMMAND_EDITOR_DELETE_SELECTION"
    Click-MenuCommand $hwnd 0 "COMMAND_SAVE_DOCUMENT"
    Assert-CliNotContains @("--db", $dbPath, "show", "1") "windows menu smoke text" | Out-Null
    Click-MenuCommand $hwnd 1 "COMMAND_EDITOR_PASTE"
    Click-MenuCommand $hwnd 0 "COMMAND_SAVE_DOCUMENT"
    Assert-CliContains @("--db", $dbPath, "show", "1") "windows menu smoke text" | Out-Null
    Send-Command $hwnd "COMMAND_EDITOR_SELECT_ALL"
    Click-MenuCommand $hwnd 1 "COMMAND_EDITOR_CUT"
    Click-MenuCommand $hwnd 0 "COMMAND_SAVE_DOCUMENT"
    Assert-CliNotContains @("--db", $dbPath, "show", "1") "windows menu smoke text" | Out-Null
    Click-MenuCommand $hwnd 1 "COMMAND_EDITOR_PASTE"
    Click-MenuCommand $hwnd 0 "COMMAND_SAVE_DOCUMENT"
    Assert-CliContains @("--db", $dbPath, "show", "1") "windows menu smoke text" | Out-Null
    Send-Command $hwnd "COMMAND_EDITOR_SELECT_ALL"
    Click-MenuCommand $hwnd 1 "COMMAND_EDITOR_DELETE_SELECTION"
    Click-MenuCommand $hwnd 1 "COMMAND_EDITOR_UNDO"
    Click-MenuCommand $hwnd 0 "COMMAND_SAVE_DOCUMENT"
    Assert-CliContains @("--db", $dbPath, "show", "1") "windows menu smoke text" | Out-Null

    $importDialog = Click-MenuCommandAndWaitDialog $hwnd 0 "COMMAND_IMPORT_TEXT" $process.Id "Import Text"
    Close-Dialog $importDialog $IDCANCEL
    $exportDialog = Click-MenuCommandAndWaitDialog $hwnd 0 "COMMAND_EXPORT_TEXT" $process.Id "Export Text"
    Close-Dialog $exportDialog $IDCANCEL
    $exportAllDialog = Click-MenuCommandAndWaitDialog $hwnd 0 "COMMAND_EXPORT_ALL_TEXT" $process.Id
    Close-Dialog $exportAllDialog $IDCANCEL

    Press-KeyChord $hwnd $tree @($VK_CONTROL_KEY, $VK_N_KEY)
    End-TreeLabelEdit $tree -Cancel
    Assert-CliContains @("--db", $dbPath, "tree") "Untitled 2" | Out-Null

    Click-MenuCommand $hwnd 2 "COMMAND_NEW_DOCUMENT"
    End-TreeLabelEdit $tree -Cancel
    Assert-CliContains @("--db", $dbPath, "tree") "Untitled 3" | Out-Null

    Click-MenuCommand $hwnd 2 "COMMAND_NEW_CHILD_DOCUMENT"
    End-TreeLabelEdit $tree "Windows Smoke Child"
    Assert-CliContains @("--db", $dbPath, "tree") "Windows Smoke Child" | Out-Null

    Click-MenuCommand $hwnd 2 "COMMAND_RENAME"
    End-TreeLabelEdit $tree "Windows Smoke Renamed"
    Assert-CliContains @("--db", $dbPath, "tree") "Windows Smoke Renamed" | Out-Null
    Press-KeyChord $hwnd $tree @($VK_F2_KEY)
    End-TreeLabelEdit $tree "Windows Smoke Shortcut Renamed"
    Assert-CliContains @("--db", $dbPath, "tree") "Windows Smoke Shortcut Renamed" | Out-Null

    Send-Command $hwnd "COMMAND_NEW_DOCUMENT"
    End-TreeLabelEdit $tree -Cancel
    Assert-MenuEnabled $hwnd "COMMAND_MOVE_UP" $true
    Click-MenuCommand $hwnd 2 "COMMAND_MOVE_UP"
    Assert-MenuEnabled $hwnd "COMMAND_MOVE_DOWN" $true
    Click-MenuCommand $hwnd 2 "COMMAND_MOVE_DOWN"

    Press-KeyChord $hwnd $tree @($VK_DELETE_KEY)
    $deleteDialog = Wait-Dialog $process.Id "j3TreeText"
    Close-Dialog $deleteDialog $IDYES
    Assert-CliContains @("--db", $dbPath, "trash") "Untitled" | Out-Null

    Click-MenuCommandAndAssertChecked $hwnd 3 "COMMAND_SHOW_TRASH"
    $restoreDialog = Click-MenuCommandAndWaitDialog $hwnd 2 "COMMAND_RESTORE" $process.Id "j3TreeText"
    Close-Dialog $restoreDialog $IDYES
    Click-MenuCommandAndAssertChecked $hwnd 3 "COMMAND_SHOW_ACTIVE_TREE"

    Send-Command $hwnd "COMMAND_NEW_DOCUMENT"
    End-TreeLabelEdit $tree -Cancel
    $deleteAgainDialog = Click-MenuCommandAndWaitDialog $hwnd 2 "COMMAND_DELETE" $process.Id "j3TreeText"
    Close-Dialog $deleteAgainDialog $IDYES
    Click-MenuCommandAndAssertChecked $hwnd 3 "COMMAND_SHOW_TRASH"
    $purgeDialog = Click-MenuCommandAndWaitDialog $hwnd 2 "COMMAND_DELETE_PERMANENTLY" $process.Id "j3TreeText"
    Close-Dialog $purgeDialog $IDYES

    $beforeWrap = ((Get-MenuItemState $hwnd "COMMAND_EDITOR_WORD_WRAP") -band $MF_CHECKED) -ne 0
    Click-MenuCommandAndAssertCheckedState $hwnd 3 "COMMAND_EDITOR_WORD_WRAP" (-not $beforeWrap)

    foreach ($command in @(
        "COMMAND_IMPORT_ENCODING_AUTO",
        "COMMAND_IMPORT_ENCODING_UTF8",
        "COMMAND_IMPORT_ENCODING_UTF8_BOM",
        "COMMAND_IMPORT_ENCODING_UTF16_BE_BOM",
        "COMMAND_IMPORT_ENCODING_KOREAN_EUC_KR",
        "COMMAND_IMPORT_ENCODING_WINDOWS_1252",
        "COMMAND_IMPORT_ENCODING_UTF16_LE_BOM"
    )) {
        Click-MenuCommandAndAssertChecked $hwnd 0 $command
    }

    foreach ($command in @(
        "COMMAND_EXPORT_ENCODING_UTF8",
        "COMMAND_EXPORT_ENCODING_UTF16_LE_BOM",
        "COMMAND_EXPORT_ENCODING_UTF16_BE_BOM",
        "COMMAND_EXPORT_ENCODING_KOREAN_EUC_KR",
        "COMMAND_EXPORT_ENCODING_WINDOWS_1252",
        "COMMAND_EXPORT_ENCODING_UTF8_BOM"
    )) {
        Click-MenuCommandAndAssertChecked $hwnd 0 $command
    }

    foreach ($command in @(
        "COMMAND_THEME_LIGHT",
        "COMMAND_THEME_CLASSIC_DARK",
        "COMMAND_THEME_SEPIA_TEAL",
        "COMMAND_THEME_GRAPHITE",
        "COMMAND_THEME_STEEL_BLUE",
        "COMMAND_THEME_FOREST"
    )) {
        Click-MenuCommandAndAssertChecked $hwnd 3 $command
    }

    Click-MenuCommandAndAssertChecked $hwnd 3 "COMMAND_LANGUAGE_KOREAN"
    Assert-TopMenuLabels $hwnd (Korean-TopMenuLabels)
    Click-MenuCommandAndAssertChecked $hwnd 3 "COMMAND_LANGUAGE_ENGLISH"
    Assert-TopMenuLabels $hwnd @("File", "Edit", "Document", "View", "Help")

    $findDialog = Click-MenuCommandAndWaitDialog $hwnd 1 "COMMAND_FIND_TEXT" $process.Id
    Assert-True ([J3Native]::IsWindow($findDialog)) "Find dialog did not open"
    $replaceDialog = Click-MenuCommandAndWaitDialog $hwnd 1 "COMMAND_REPLACE_TEXT" $process.Id
    Assert-True ([J3Native]::IsWindow($replaceDialog)) "Replace dialog did not open"
    [void][J3Native]::PostMessage($replaceDialog, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    Wait-Until "replace dialog close" { -not [J3Native]::IsWindow($replaceDialog) } 5000 | Out-Null

    $fontDialog = Click-MenuCommandAndWaitDialog $hwnd 3 "COMMAND_EDITOR_FONT" $process.Id
    Close-Dialog $fontDialog $IDCANCEL
    $aboutDialog = Click-MenuCommandAndWaitDialog $hwnd 4 "COMMAND_ABOUT" $process.Id "About j3TreeText"
    Close-Dialog $aboutDialog $IDOK

    if (Test-MenuEnabled $hwnd "COMMAND_CLOSE_TAB") {
        Click-MenuCommand $hwnd 0 "COMMAND_CLOSE_TAB"
    }
    if (Test-MenuEnabled $hwnd "COMMAND_CLOSE_TAB") {
        Press-KeyChord $hwnd $hwnd @($VK_CONTROL_KEY, $VK_W_KEY)
    }
    for ($closeIndex = 0; $closeIndex -lt 10 -and (Test-MenuEnabled $hwnd "COMMAND_CLOSE_TAB"); $closeIndex++) {
        Send-Command $hwnd "COMMAND_CLOSE_TAB"
    }
    Assert-MenuEnabled $hwnd "COMMAND_CLOSE_TAB" $false

    Click-MenuCommandAndWaitProcessExit $hwnd 0 "COMMAND_CLOSE_WINDOW" $process

    $process = $null

    $restart = Start-Process -FilePath $guiExe -WorkingDirectory $tempRoot -RedirectStandardError (Join-Path $tempRoot "gui-restart-stderr.log") -PassThru
    try {
        $restartHwnd = Wait-Until "restarted main window" { Find-AppWindow $restart.Id } 10000
        Assert-TopMenuLabels $restartHwnd @("File", "Edit", "Document", "View", "Help")
        Assert-MenuChecked $restartHwnd "COMMAND_IMPORT_ENCODING_UTF16_LE_BOM" $true
        Assert-MenuChecked $restartHwnd "COMMAND_EXPORT_ENCODING_UTF8_BOM" $true
        Assert-MenuChecked $restartHwnd "COMMAND_THEME_FOREST" $true
        Assert-MenuChecked $restartHwnd "COMMAND_LANGUAGE_ENGLISH" $true
        Send-Command $restartHwnd "COMMAND_CLOSE_WINDOW"
        Wait-Until "restart process exit" { $restart.Refresh(); $restart.HasExited } 8000 | Out-Null
    } finally {
        if ($restart -and -not $restart.HasExited) {
            $restart.Kill()
        }
    }

    $stderr = if (Test-Path -LiteralPath $stderrPath) { Get-Content -LiteralPath $stderrPath -Raw } else { "" }
    Assert-True ($stderr -notmatch "panic|RefCell already borrowed|thread '") "GUI stderr contains a panic-like message:`n$stderr"

    $script:SmokeSucceeded = $true
    [pscustomobject]@{
        Result = "ok"
        TempRoot = $tempRoot
        Database = $dbPath
        Notes = "Executed Win32 menu click, shortcut, WM_COMMAND, dialog, DB state, and restart-restore smoke paths."
    } | Format-List
} finally {
    if ($process -and -not $process.HasExited) {
        $process.Kill()
    }
    if ($KeepTemp -or -not $script:SmokeSucceeded) {
        Write-Host "Temp artifacts kept for inspection: $tempRoot"
    } elseif (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}
