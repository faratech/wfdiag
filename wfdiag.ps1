$version = "2.0.6"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.IO.Compression.FileSystem
$script:stopScript = $false

$currentPrincipal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
$isAdmin = $currentPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (!$isAdmin) {
    [System.Windows.Forms.MessageBox]::Show("Admin rights are needed for some reports including BSOD Minidump. Running as a standard user may limit results.", "Admin Rights Required", [System.Windows.Forms.MessageBoxButtons]::OK, [System.Windows.Forms.MessageBoxIcon]::Information, [System.Windows.Forms.MessageBoxDefaultButton]::Button1, [System.Windows.Forms.MessageBoxOptions]::ServiceNotification)
}
# Setup paths
$desktopPath = [Environment]::GetFolderPath("Desktop")
$filePath = Join-Path -Path $desktopPath -ChildPath "WindowsForum"
$zipFilePath = Join-Path -Path $desktopPath -ChildPath "WF-Diag.zip"
$minidumpPath = Join-Path -Path $desktopPath\WindowsForum -ChildPath "Minidump"

# Ensure the directory exists
if (Test-Path -Path $filePath) {
    Remove-Item -Path $filePath -Recurse -Force
}
if (Test-Path -Path $zipFilePath) {
    Remove-Item -Path $zipFilePath -Recurse -Force
}
New-Item -ItemType Directory -Path $filePath -Force
New-Item -ItemType Directory -Path $filePath -Force
New-Item -ItemType Directory -Path $minidumpPath -Force
    $diagnosticTasks = @(

            @{ Name = "Comp System"; Task = { param($filePath) Get-CimInstance -ClassName Win32_ComputerSystem | Out-File "$filePath\WindowsForum-CompSystem.txt" }},
            @{ Name = "OS"; Task = { param($filePath) Get-CimInstance -ClassName Win32_OperatingSystem | Out-File "$filePath\WindowsForum-OS.txt" }},
            @{ Name = "BIOS"; Task = { param($filePath) Get-CimInstance -ClassName Win32_BIOS | Out-File "$filePath\WindowsForum-BIOS.txt" }},
            @{ Name = "BaseBoard"; Task = { param($filePath) Get-CimInstance -ClassName Win32_BaseBoard | Out-File "$filePath\WindowsForum-BaseBoard.txt" }},
            @{ Name = "Processor"; Task = { param($filePath) Get-CimInstance -ClassName Win32_Processor | Out-File "$filePath\WindowsForum-Processor.txt" }},
            @{ Name = "Physical Mem"; Task = { param($filePath) Get-CimInstance -ClassName Win32_PhysicalMemory | Out-File "$filePath\WindowsForum-PhysicalMemory.txt" }},
            @{ Name = "Dev Mem Addr"; Task = { param($filePath) Get-CimInstance -ClassName Win32_DeviceMemoryAddress | Out-File "$filePath\WindowsForum-DevMemAddr.txt" }},
            @{ Name = "DMA Channel"; Task = { param($filePath) Get-CimInstance -ClassName Win32_DMAChannel | Out-File "$filePath\WindowsForum-DMAChannel.txt" }},
            @{ Name = "IRQ Resource"; Task = { param($filePath) Get-CimInstance -ClassName Win32_IRQResource | Out-File "$filePath\WindowsForum-IRQResource.txt" }},
            @{ Name = "Disk Drive"; Task = { param($filePath) Get-CimInstance -ClassName Win32_DiskDrive | Out-File "$filePath\WindowsForum-DiskDrive.txt" }},
            @{ Name = "Disk Partition"; Task = { param($filePath) Get-CimInstance -ClassName Win32_DiskPartition | Out-File "$filePath\WindowsForum-DiskPartition.txt" }},
            @{ Name = "Sys Devices"; Task = { param($filePath) Get-CimInstance -ClassName Win32_SystemDevices | Out-File "$filePath\WindowsForum-SysDevices.txt" }},
            @{ Name = "Net Adapter"; Task = { param($filePath) Get-CimInstance -ClassName Win32_NetworkAdapter | Out-File "$filePath\WindowsForum-NetAdapter.txt" }},
            @{ Name = "Printer"; Task = { param($filePath) Get-CimInstance -ClassName Win32_Printer | Out-File "$filePath\WindowsForum-Printer.txt" }},
            @{ Name = "Environment"; Task = { param($filePath) Get-CimInstance -ClassName Win32_Environment | Out-File "$filePath\WindowsForum-Environment.txt" }},
            @{ Name = "Startup Cmd"; Task = { param($filePath) Get-CimInstance -ClassName Win32_StartupCommand | Out-File "$filePath\WindowsForum-StartupCmd.txt" }},
            @{ Name = "Sys Driver"; Task = { param($filePath) Get-CimInstance -ClassName Win32_SystemDriver | Out-File "$filePath\WindowsForum-SysDriver.txt" }},
        @{ Name = "DXDiag"; Task = { param($filePath) dxdiag /t "$filePath\WindowsForum-DxDiag.txt" "/whql:off" }},
        @{ Name = "SystemInfo"; Task = { param($filePath) systeminfo | Out-File "$filePath\WindowsForum-SystemInfo.txt" }},
        @{ Name = "Drivers"; Task = { param($filePath) Get-CimInstance -ClassName Win32_PnPSignedDriver | Select-Object DeviceName, DriverVersion, Manufacturer | Out-File "$filePath\WindowsForum-DriversList.txt" }},
        @{ Name = "Event Logs"; Task = { param($filePath) $logNames = "System", "Application"; foreach ($log in $logNames) {$logPath = "$filePath\WindowsForum-$log.evtx"; wevtutil epl $log $logPath }}},
        @{ Name = "IPConfig"; Task = { param($filePath) ipconfig /all | Out-File "$filePath\WindowsForum-NetworkConfig.txt" }},
        @{ Name = "Installed Programs"; Task = { param($filePath) Get-Package | Select-Object Name, Version | Out-File "$filePath\WindowsForum-InstalledPrograms.txt" }},
        @{ Name = "Windows Store Apps"; Task = { param($filePath) Get-AppxPackage | Select-Object Name, Version | Out-File "$filePath\WindowsForum-StoreApps.txt" }},
        @{ Name = "System Services"; Task = { param($filePath) Get-Service | Out-File "$filePath\WindowsForum-SystemServices.txt" }},
        @{ Name = "Processes"; Task = { param($filePath) Get-Process | Out-File "$filePath\WindowsForum-RunningProcesses.txt" }},
        @{ Name = "Performance Data"; Task = { param($filePath) Get-Counter | Out-File "$filePath\WindowsForum-PerformanceData.txt" }},
        @{ Name = "HOSTS File"; Task = { param($filePath) Copy-Item "$env:windir\System32\drivers\etc\hosts" "$filePath\WindowsForum-HostsFile.txt" }},
        @{ Name = "Dsregcmd"; Task = { param($filePath) dsregcmd /status | Out-File "$filePath\WindowsForum-DsRegCmd.txt" }},
        @{ Name = "Scheduled Tasks"; Task = { param($filePath) Get-ScheduledTask | Out-File "$filePath\WindowsForum-ScheduledTasks.txt" }},
        @{ Name = "Windows Update Log"; Task = { param($filePath) wevtutil qe Microsoft-Windows-WindowsUpdateClient/Operational /f:text > "$filePath\WindowsForum-WindowsUpdate.txt" }}
    )
    # Add admin tasks if running with admin privileges
if ($isAdmin) {
    $diagnosticTasks += @(
        @{ Name = "Read-Only Chkdsk"; Task = { param($filePath, $zipFilePath) Repair-Volume -DriveLetter C -Scan -Verbose | Out-File "$filePath\WindowsForum-Chkdsk.txt" }}
        @{ Name = "DISM CheckHealth"; Task = { param($filePath, $zipFilePath) Start-Process dism -ArgumentList "/online /cleanup-image /checkhealth > $filePath\WindowsForum-DISMCheckHealth.txt" }}
        #        @{ Name = "MSINFO32"; Task = { param($filePath, $zipFilePath) Start-Process msinfo32 -ArgumentList "/nfo $filePath\WindowsForum-MSInfo32.nfo" -Wait }},
        @{ Name = "Battery Report"; Task = { param($filePath, $zipFilePath) Start-Process powercfg -ArgumentList "/batteryreport /output `"$filePath\WindowsForum-BatteryReport.html`"" -NoNewWindow -Wait }},
#        @{ Name = "Windows Update Log"; Task = { param($filePath, $zipFilePath) Get-WindowsUpdateLog | Out-File "$filePath\WindowsForum-WindowsUpdateLog.txt" }},
        @{ Name = "Driver Verifier"; Task = { param($filePath, $zipFilePath) verifier /querysettings | Out-File "$filePath\WindowsForum-DriverVerifierSettings.txt" }},
        @{ Name = "BSOD Minidump"; Task = { param($filePath, $zipFilePath) $minidumpPath = "C:\Windows\Minidump\*"
            $destination = Join-Path -Path $filePath -ChildPath "Minidump"
            New-Item -ItemType Directory -Path $filePath\Minidump -Force
                    if (Test-Path -Path $minidumpPath) {
                        # Grant access to the minidump files
                        Get-ChildItem -Path $minidumpPath -Recurse | ForEach-Object {
                             $acl = Get-Acl -Path $_.FullName
                             $permission = "BUILTIN\Administrators","FullControl","Allow"
                             $accessRule = New-Object System.Security.AccessControl.FileSystemAccessRule $permission
                             $acl.SetAccessRule($accessRule)
                             Set-Acl -Path $_.FullName -AclObject $acl -Recurse
                         }
                    Get-ChildItem -Path $minidumpPath -Filter "*.dmp" | Sort-Object LastWriteTime -Descending | Select-Object -First 3 | ForEach-Object {
                    Copy-Item -Path $_.FullName -Destination $destination -Force
                }
                    }
                }}
            )
}

# Create and configure the progress bar form
$form = New-Object System.Windows.Forms.Form
$form.Text = "WindowsForum.com Diagnostic Tool $($version)"
$form.StartPosition = 'CenterScreen'
$form.TopMost = $true
$form.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::FixedSingle
$form.MaximizeBox = $false
$form.MinimizeBox = $false
$form.Size = New-Object System.Drawing.Size(500, 200) # Adjust the size of the form here

$label = New-Object System.Windows.Forms.Label
$label.Location = New-Object System.Drawing.Point(10, 50)
$label.Size = New-Object System.Drawing.Size(450, 140) # Adjust the size of the label here
$label.Text = "Initializing..."
$form.Controls.Add($label)

# Create and configure the progress bar
$progressBar = New-Object System.Windows.Forms.ProgressBar
$progressBar.Location = New-Object System.Drawing.Point(10, 10)
$progressBar.Size = New-Object System.Drawing.Size(450, 20)
$progressBar.Style = [System.Windows.Forms.ProgressBarStyle]::Continuous
$form.Controls.Add($progressBar)

# Show the form
$form.Show()
$form.Activate()
$form.Refresh()

# Start the background work
#$job = Start-Job -ScriptBlock $backgroundWork -ArgumentList $form, $progressBar, $label, $filePath, $zipFilePath
# Run tasks with progress bar

# Handle the FormClosing event
$form.Add_FormClosing({
    param($sender, $e)
    try {
        $label.Text = "Cancelling all tasks and closing..."
        $jobs | Stop-Job -ErrorAction SilentlyContinue | Remove-Job -Force -ErrorAction SilentlyContinue
        Start-Job -ScriptBlock { Get-Process -Name dxdiag -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue }
        $script:stopScript = $true
    #    $form.Close()
    } catch {
        Write-Host "Error stopping jobs or msinfo32 process: $_" }
})

$totalTasks = $diagnosticTasks.Count
$currentTask = 0
$jobs = @()
$currentTaskIndex = 0
foreach ($task in $diagnosticTasks) {
    $jobs += Start-Job -ScriptBlock $task.Task -ArgumentList $filePath, $zipFilePath
    $progressBar.Value = ($currentTaskIndex / $totalTasks) * 100
    $label.Text = "Exporting Logs... $($diagnosticTasks[$currentTaskIndex].Name) ($currentTaskIndex of $totalTasks)"
    [System.Windows.Forms.Application]::DoEvents()
    $currentTaskIndex++
}

while (($jobs.State -contains 'Running') -and !$script:stopScript) {
    if ($form.IsDisposed -or $script:stopScript) {
        $jobs | ForEach-Object { Stop-Job -Id $_.Id -ErrorAction SilentlyContinue}
        break
    }
    $runningJobs = $jobs | Where-Object { $_.State -eq 'Running' }
    $runningTasks = $runningJobs | ForEach-Object { $diagnosticTasks[$jobs.IndexOf($_)].Name }
    $remainingTasks = $runningJobs.Count
    $completedTasks = $totalTasks - $remainingTasks
    $progressBar.Value = ($completedTasks / $totalTasks) * 100
    $label.Text = "Running Diagnostics... This will take awhile... completed $completedTasks of $totalTasks. $($runningTasks -join ', ')"
    [System.Windows.Forms.Application]::DoEvents()
    Start-Sleep -Seconds 1
}
# Check if the script was stopped
if ($script:stopScript) {
    $label.Text = "Stopped. Cancelling all tasks and closing..."
    $form.Refresh()
    # Stop all running jobs asynchronously
    $jobs | Where-Object { $_.State -eq 'Running' } | Stop-Job -PassThru | Receive-Job -Wait -AutoRemoveJob
    Get-Job | Remove-Job -Force
    Start-Job -ScriptBlock { Get-Process -Name dxdiag -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue }
    $form.Close()
    exit
}
else {
# Wait for all jobs to complete
$jobs | Wait-Job
Get-Job | Remove-Job -Force
# Compress results
[System.IO.Compression.ZipFile]::CreateFromDirectory($filePath, $zipFilePath)
$label.Text = "Diagnostics complete. Results have been saved to $zipFilePath. Visit WindowsForum.com!"
$progressBar.Value = 100
$form.Refresh()
Start-Sleep -Seconds 2
#Start-Process "https://windowsforum.com"
# Open the .zip file
Invoke-Item -Path $zipFilePath
$form.Close()
# Show a popup window with the location of the zip file
[System.Windows.Forms.MessageBox]::Show("Results have been saved to $zipFilePath. ", "Log Collection Complete", [System.Windows.Forms.MessageBoxButtons]::OK, [System.Windows.Forms.MessageBoxIcon]::Information, [System.Windows.Forms.MessageBoxDefaultButton]::Button1, [System.Windows.Forms.MessageBoxOptions]::ServiceNotification)
}$allOutput = @()

# Your existing code here...
###NOT WORKING - COMBINE INTO HTML
foreach ($task in $diagnosticTasks) {
    $job = Start-Job -ScriptBlock $task.Task -ArgumentList $filePath, $zipFilePath
    $jobs += $job
    $result = Receive-Job -Job $job -Wait
    $allOutput += New-Object PSObject -Property @{
        'Task' = $task.Name
        'Output' = $result
    }
    # Your existing code here...
}

# Your existing code here...

# Convert all output to HTML
$html = $allOutput | ConvertTo-Html -Property Task, Output
# Save HTML to file
$html | Out-File "$filePath\WindowsForum-AllOutput.html"

# Your existing code here...
