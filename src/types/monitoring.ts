/**
 * Shared type definitions for system monitoring
 * Used by SystemMonitoring.tsx and ProcessesTab.tsx
 */

export interface DiskInfo {
  name: string
  mount_point: string
  total_gb: number
  used_gb: number
  available_gb: number
  utilization: number
  file_system: string
  disk_type: string
}

/**
 * Full ProcessInfo interface matching the Rust backend
 * Includes all 15 columns for htop-like display plus tree view fields
 */
export interface ProcessInfo {
  pid: number
  parent_pid: number
  name: string
  exe_path: string
  command: string
  user: string
  cpu_percent: number
  memory_percent: number
  memory_mb: number
  virtual_memory_mb: number
  shared_memory_mb: number
  cpu_time_secs: number
  start_time: number
  status: string
  thread_count: number
  handle_count: number
  priority: number
  io_read_bytes: number
  io_write_bytes: number
  is_elevated: boolean
  efficiency_mode: boolean
  arch: string
  // Tree view fields
  tree_depth: number
  tree_prefix: string
  has_children: boolean
  is_collapsed: boolean
  // Legacy fields for compatibility with simpler views
  disk_read_bytes?: number
  disk_write_bytes?: number
}

export interface NetworkConnection {
  protocol: string
  local_addr: string
  remote_addr: string
  status: string
}

export interface SystemStats {
  cpu_utilization: number
  per_cpu_utilization: number[]
  cpu_frequency: number
  memory_total_gb: number
  memory_used_gb: number
  memory_available_gb: number
  memory_utilization: number
  swap_total_gb: number
  swap_used_gb: number
  swap_utilization: number
  disk_utilization: number
  disk_read_bytes: number
  disk_write_bytes: number
  disks: DiskInfo[]
  network_upload_kb: number
  network_download_kb: number
  npu_available: boolean
  npu_name: string | null
  npu_utilization: number | null
  top_processes: ProcessInfo[]
  timestamp: number
}
