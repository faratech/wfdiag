# WFDiag Backend API

A high-performance Windows diagnostic tool backend written in Rust that provides both REST API and CLI interfaces.

## Features

- 🚀 REST API with real-time WebSocket updates
- 📊 JSON output for easy integration
- 🔧 CLI mode for scripting and automation
- 🛡️ Admin privilege detection and handling
- 📦 Multiple output formats (JSON, ZIP)
- ⚡ Async/concurrent task execution

## Usage

### CLI Mode

```bash
# List all available diagnostic tasks
wfdiag-backend list

# Run all diagnostics and output JSON
wfdiag-backend run

# Run specific tasks
wfdiag-backend run --tasks "computer_system,operating_system,processor"

# Output as ZIP file
wfdiag-backend run --format zip
```

### Server Mode

```bash
# Start server on default port (8080)
wfdiag-backend server

# Start on custom host/port
wfdiag-backend server --host 0.0.0.0 --port 3000
```

## API Endpoints

### GET /api/v1/system
Get current system information.

**Response Schema:**
```typescript
{
  success: boolean;
  data?: {
    os_version: string;        // e.g., "Windows 11 Pro Version 23H2 (Build 22631.3958)"
    computer_name: string;     // e.g., "DESKTOP-ABC123"
    username: string;          // e.g., "john.doe"
    is_admin: boolean;         // true if running with admin privileges
    cpu_info: string;          // e.g., "Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz"
    total_memory_gb: number;   // e.g., 31.92
    available_memory_gb: number; // e.g., 16.5
  };
  error?: string;              // Only present if success is false
}
```

**Example Response:**
```json
{
  "success": true,
  "data": {
    "os_version": "Windows 11 Pro Version 23H2 (Build 22631.3958)",
    "computer_name": "DESKTOP-ABC123",
    "username": "john.doe",
    "is_admin": true,
    "cpu_info": "Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz",
    "total_memory_gb": 31.92,
    "available_memory_gb": 16.5
  }
}
```

### GET /api/v1/tasks
Get available diagnostic tasks.

**Response Schema:**
```typescript
{
  success: boolean;
  data?: Array<{
    id: string;              // Unique identifier (lowercase, underscored)
    name: string;            // Display name
    description: string;     // Brief description of what the task does
    admin_required: boolean; // Whether admin privileges are needed
    category: string;        // Task category: "System" | "Hardware" | "Network" | "Storage" | "Services" | "Logs" | "Drivers" | "Other"
  }>;
  error?: string;
}
```

**Example Response:**
```json
{
  "success": true,
  "data": [
    {
      "id": "computer_system",
      "name": "Computer System",
      "description": "Hardware and system information",
      "admin_required": false,
      "category": "System"
    },
    {
      "id": "operating_system",
      "name": "Operating System",
      "description": "Windows version and configuration",
      "admin_required": false,
      "category": "System"
    },
    {
      "id": "bsod_minidump",
      "name": "BSOD Minidump",
      "description": "Blue Screen of Death crash dumps",
      "admin_required": true,
      "category": "Logs"
    }
  ]
}
```

### POST /api/v1/diagnostics
Start a new diagnostic session.

**Request Schema:**
```typescript
{
  selected_tasks: string[];      // Array of task IDs to run
  output_format?: "json" | "zip" | "both"; // Default: "both"
}
```

**Response Schema:**
```typescript
{
  success: boolean;
  data?: {
    id: string;                  // UUID of the session
    status: "pending" | "running" | "completed" | "failed" | "cancelled";
    progress: number;            // 0.0 to 1.0
    current_task: string | null; // Currently executing task name
    completed_tasks: number;     // Number of completed tasks
    total_tasks: number;         // Total number of tasks to run
    started_at: string;          // ISO 8601 timestamp
    completed_at: string | null; // ISO 8601 timestamp when finished
    output_path: string | null;  // Path to output file when completed
    errors: string[];            // Array of error messages
  };
  error?: string;
}
```

**Example Request:**
```json
{
  "selected_tasks": ["computer_system", "operating_system", "processor"],
  "output_format": "zip"
}
```

**Example Response:**
```json
{
  "success": true,
  "data": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "status": "pending",
    "progress": 0.0,
    "current_task": null,
    "completed_tasks": 0,
    "total_tasks": 3,
    "started_at": "2024-01-15T10:30:00.123Z",
    "completed_at": null,
    "output_path": null,
    "errors": []
  }
}
```

### GET /api/v1/diagnostics/{session_id}
Get session status.

**Response Schema:** Same as POST /api/v1/diagnostics response

**Example Response (Running):**
```json
{
  "success": true,
  "data": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "status": "running",
    "progress": 0.33,
    "current_task": "Operating System",
    "completed_tasks": 1,
    "total_tasks": 3,
    "started_at": "2024-01-15T10:30:00.123Z",
    "completed_at": null,
    "output_path": null,
    "errors": []
  }
}
```

**Example Response (Completed):**
```json
{
  "success": true,
  "data": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "status": "completed",
    "progress": 1.0,
    "current_task": null,
    "completed_tasks": 3,
    "total_tasks": 3,
    "started_at": "2024-01-15T10:30:00.123Z",
    "completed_at": "2024-01-15T10:31:45.678Z",
    "output_path": "C:\\Users\\john\\Desktop\\WF-Diag_550e8400-e29b-41d4-a716-446655440000.zip",
    "errors": []
  }
}
```

### POST /api/v1/diagnostics/{session_id}/cancel
Cancel a running session.

**Response Schema:**
```typescript
{
  success: boolean;
  data?: string;    // Success message
  error?: string;   // Error message if cancellation failed
}
```

**Example Response:**
```json
{
  "success": true,
  "data": "Session cancelled"
}
```

### GET /api/v1/diagnostics/{session_id}/download
Download session results as ZIP.

**Response:** Binary ZIP file with appropriate headers
- Content-Type: `application/zip`
- Content-Disposition: `attachment; filename="WF-Diag_{session_id}.zip"`

**Error Response (if file not found):**
```json
{
  "success": false,
  "error": "Results file not found"
}
```

## WebSocket Events

Connect to `/ws` for real-time progress updates:

**Progress Update Schema:**
```typescript
{
  session_id: string;           // UUID of the session
  progress: number;             // 0.0 to 1.0
  status: "pending" | "running" | "completed" | "failed" | "cancelled";
  current_task: string | null;  // Currently executing task
  message: string;              // Human-readable status message
  completed_tasks: number;      // Number of completed tasks
  total_tasks: number;          // Total number of tasks
  timestamp: string;            // ISO 8601 timestamp
}
```

**Example WebSocket Message:**
```json
{
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "progress": 0.66,
  "status": "running",
  "current_task": "Processor",
  "message": "Running Processor...",
  "completed_tasks": 2,
  "total_tasks": 3,
  "timestamp": "2024-01-15T10:30:45.123Z"
}

```

**WebSocket Connection Example:**
```javascript
const ws = new WebSocket('ws://localhost:8080/ws');

ws.onopen = () => {
  console.log('Connected to real-time updates');
  // Optional: Subscribe to specific session
  ws.send(JSON.stringify({ 
    type: 'subscribe', 
    session_id: '550e8400-e29b-41d4-a716-446655440000' 
  }));
};

ws.onmessage = (event) => {
  const update = JSON.parse(event.data);
  console.log(`Progress: ${(update.progress * 100).toFixed(1)}% - ${update.message}`);
  
  if (update.status === 'completed') {
    console.log('Diagnostics completed!');
  }
};

ws.onerror = (error) => {
  console.error('WebSocket error:', error);
};

ws.onclose = () => {
  console.log('Disconnected from real-time updates');
  // Implement reconnection logic if needed
};
```

## Common Data Types

### Task Categories
```typescript
type TaskCategory = 
  | "System"     // OS and computer information
  | "Hardware"   // CPU, Memory, Motherboard
  | "Network"    // Network adapters and configuration
  | "Storage"    // Disks and partitions
  | "Services"   // Windows services and processes
  | "Logs"       // Event logs and crash dumps
  | "Drivers"    // Device drivers and verification
  | "Other";     // Miscellaneous diagnostics
```

### Session Status
```typescript
type SessionStatus = 
  | "pending"    // Session created but not started
  | "running"    // Currently executing tasks
  | "completed"  // All tasks finished successfully
  | "failed"     // One or more tasks failed
  | "cancelled"; // User cancelled the session
```

### Output Formats
```typescript
type OutputFormat = 
  | "json"       // JSON file with structured data
  | "zip"        // ZIP file with text reports
  | "both";      // Both JSON and ZIP (default)
```

## Error Handling

All API responses follow this pattern:

**Success Response:**
```json
{
  "success": true,
  "data": { /* Response data */ }
}
```

**Error Response:**
```json
{
  "success": false,
  "error": "Error message describing what went wrong"
}
```

### Common Error Scenarios

1. **Invalid Task IDs:**
```json
{
  "success": false,
  "error": "Unknown task IDs: invalid_task_1, invalid_task_2"
}
```

2. **Session Not Found:**
```json
{
  "success": false,
  "error": "Session not found"
}
```

3. **Admin Required:**
```json
{
  "success": false,
  "error": "Admin privileges required for tasks: bsod_minidump, driver_verifier"
}
```

4. **Already Running:**
```json
{
  "success": false,
  "error": "Diagnostics already running"
}

## Integration Examples

### PowerShell
```powershell
# Run diagnostics
$response = Invoke-RestMethod -Uri "http://localhost:8080/api/v1/diagnostics" `
  -Method POST `
  -ContentType "application/json" `
  -Body '{"selected_tasks": ["computer_system"]}'

# Check status
$status = Invoke-RestMethod -Uri "http://localhost:8080/api/v1/diagnostics/$($response.data.id)"
```

### Python
```python
import requests
import json

# Start diagnostics
response = requests.post('http://localhost:8080/api/v1/diagnostics', 
    json={'selected_tasks': ['computer_system', 'operating_system']})
session = response.json()['data']

# Poll for completion
import time
while True:
    status = requests.get(f'http://localhost:8080/api/v1/diagnostics/{session["id"]}').json()
    if status['data']['status'] in ['completed', 'failed']:
        break
    time.sleep(1)
```

### JavaScript/Node.js
```javascript
// Using fetch API
const response = await fetch('http://localhost:8080/api/v1/diagnostics', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ selected_tasks: ['computer_system'] })
});

const { data: session } = await response.json();
```

## Building

```bash
# Development build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test
```

## Frontend Integration

This backend can be used with any frontend framework:
- **React/Vue/Angular**: Use fetch/axios for REST API
- **.NET WinUI3/MAUI**: Use HttpClient
- **Flutter**: Use http package
- **Tauri**: Use as sidecar process

## Available Diagnostic Tasks

Here's a complete list of all diagnostic task IDs that can be used in the `selected_tasks` array:

### System Information
- `computer_system` - Hardware and system information
- `operating_system` - Windows version and configuration
- `bios` - Firmware and boot settings
- `baseboard` - Motherboard specifications
- `environment` - Environment variables
- `startup_command` - Startup programs

### Hardware
- `processor` - CPU details and capabilities
- `physical_memory` - RAM configuration and usage
- `device_memory_address` - Memory address ranges
- `dma_channel` - DMA channel information
- `irq_resource` - IRQ resource allocation

### Storage
- `disk_drive` - Physical disk information
- `disk_partition` - Partition layout and sizes
- `chkdsk` ⚠️ - Disk integrity check (admin required)

### Network
- `network_adapter` - Network interfaces and settings
- `ipconfig` - IP configuration details

### Devices & Drivers
- `system_devices` - All system devices
- `system_driver` - Installed system drivers
- `drivers` - Detailed driver information
- `printer` - Installed printers
- `dxdiag` - DirectX diagnostics
- `driver_verifier` ⚠️ - Driver verifier status (admin required)

### Services & Processes
- `system_services` - Windows services status
- `processes` - Running processes with CPU/memory usage
- `scheduled_tasks` - Task scheduler entries

### Logs & Reports
- `event_logs` - System and application event logs
- `windows_update_log` - Windows Update history
- `bsod_minidump` ⚠️ - Blue screen crash dumps (admin required)

### Performance & Health
- `performance_data` - Performance counter data
- `systeminfo` - Comprehensive system summary
- `battery_report` ⚠️ - Battery health report (admin required)
- `dism_checkhealth` ⚠️ - System image health (admin required)

### Other
- `hosts_file` - HOSTS file contents
- `dsregcmd` - Domain/Azure AD join status
- `installed_programs` - List of installed software
- `windows_store_apps` - Microsoft Store applications

⚠️ = Requires administrator privileges

## Security Notes

- The backend detects admin privileges automatically
- CORS is enabled for development (configure for production)
- File paths are sanitized to prevent directory traversal
- Session IDs use UUIDs for security
- Sensitive information in diagnostic outputs should be reviewed before sharing