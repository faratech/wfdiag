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

**Response:**
```json
{
  "success": true,
  "data": {
    "os_version": "Windows 11 Pro",
    "computer_name": "DESKTOP-ABC123",
    "username": "user",
    "is_admin": true,
    "cpu_info": "Intel Core i7-9700K",
    "total_memory_gb": 32.0,
    "available_memory_gb": 16.5
  }
}
```

### GET /api/v1/tasks
Get available diagnostic tasks.

**Response:**
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
    }
  ]
}
```

### POST /api/v1/diagnostics
Start a new diagnostic session.

**Request:**
```json
{
  "selected_tasks": ["computer_system", "operating_system"],
  "output_format": "both"
}
```

**Response:**
```json
{
  "success": true,
  "data": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "status": "pending",
    "progress": 0.0,
    "total_tasks": 2,
    "started_at": "2024-01-15T10:30:00Z"
  }
}
```

### GET /api/v1/diagnostics/{session_id}
Get session status.

### POST /api/v1/diagnostics/{session_id}/cancel
Cancel a running session.

### GET /api/v1/diagnostics/{session_id}/download
Download session results as ZIP.

## WebSocket

Connect to `/ws` for real-time progress updates:

```javascript
const ws = new WebSocket('ws://localhost:8080/ws');

ws.onmessage = (event) => {
  const update = JSON.parse(event.data);
  console.log(`Progress: ${update.progress * 100}% - ${update.message}`);
};
```

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

## Security Notes

- The backend detects admin privileges automatically
- CORS is enabled for development (configure for production)
- File paths are sanitized to prevent directory traversal
- Session IDs use UUIDs for security