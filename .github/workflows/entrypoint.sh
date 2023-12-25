#!/bin/bash

curl -d '{"method": "force", "jsonrpc": "2.0", "id":0, "params":{} }' -H "Content-Type: application/json" -X POST http://43.154.77.82:8010/api/v2/forceschedulers/force

