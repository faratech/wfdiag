// Maps real DiagnosticTask ids / categories to Font Awesome classes.
// Ported from the theme-demo task list (data.jsx). The real backend
// DiagnosticTask has no icon field, so this adapter supplies it.

const TASK_ICON: Record<string, string> = {
  comp_system: 'fa-desktop',
  os_info: 'fa-brands fa-windows',
  bios: 'fa-microchip',
  systeminfo: 'fa-circle-info',
  environment: 'fa-terminal',
  startup_command: 'fa-play',
  services: 'fa-gears',
  processes: 'fa-list',
  scheduled_tasks: 'fa-clock',
  dsregcmd: 'fa-building-shield',
  dism_health: 'fa-shield-halved',
  baseboard: 'fa-microchip',
  processor: 'fa-microchip',
  physical_memory: 'fa-memory',
  device_memory: 'fa-memory',
  dma_channel: 'fa-network-wired',
  irq_resource: 'fa-bolt',
  npu_info: 'fa-brain',
  system_devices: 'fa-server',
  printer: 'fa-print',
  battery_report: 'fa-battery-three-quarters',
  disk_drive: 'fa-hard-drive',
  disk_partition: 'fa-table-cells',
  logical_disk: 'fa-floppy-disk',
  disk_fragmentation: 'fa-puzzle-piece',
  chkdsk: 'fa-stethoscope',
  network_adapter: 'fa-ethernet',
  ipconfig: 'fa-network-wired',
  hosts_file: 'fa-file-lines',
  system_driver: 'fa-screwdriver-wrench',
  drivers_list: 'fa-list',
  driver_verifier: 'fa-circle-check',
  dxdiag: 'fa-display',
  installed_programs: 'fa-cube',
  store_apps: 'fa-store',
  performance: 'fa-gauge-high',
  firewall_status: 'fa-shield',
  event_logs: 'fa-rectangle-list',
  windows_update: 'fa-arrows-rotate',
  minidump: 'fa-bug',
}

const CATEGORY_ICON: Record<string, string> = {
  System: 'fa-desktop',
  Hardware: 'fa-microchip',
  Storage: 'fa-hard-drive',
  Network: 'fa-network-wired',
  Drivers: 'fa-screwdriver-wrench',
  Graphics: 'fa-display',
  Software: 'fa-cube',
  Performance: 'fa-gauge-high',
  Security: 'fa-shield',
  Logs: 'fa-rectangle-list',
  Debug: 'fa-bug',
}

/** Returns a Font Awesome class string (without the leading `fa-solid`). */
export function taskIcon(id: string, category?: string): string {
  return TASK_ICON[id] || (category && CATEGORY_ICON[category]) || 'fa-circle-dot'
}

export const NAV_TAB_ICON: Record<string, string> = {
  diagnostics: 'fa-stethoscope',
  monitoring: 'fa-chart-line',
  processes: 'fa-microchip',
  ai: 'fa-wand-magic-sparkles',
  issues: 'fa-shield-halved',
  history: 'fa-clock-rotate-left',
}
