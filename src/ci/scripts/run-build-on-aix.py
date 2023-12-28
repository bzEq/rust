#!/usr/bin/env python3
# -*- coding: utf-8 -*-

import urllib.request
import sys
import time
import json

REMOTE_AIX_BUILDBOT_URL = 'http://43.154.77.82:8010/api/v2'


def RequestBuild():
    body = {
        'method': 'force',
        'jsonrpc': '2.0',
        'id': 0,
        'params': {},
    }
    req = urllib.request.Request(
        url=REMOTE_AIX_BUILDBOT_URL + '/forceschedulers/force',
        method='POST',
    )
    req.add_header('Content-Type', 'application/json')
    payload = json.dumps(body).encode('utf-8')
    req.add_header('Content-Length', len(payload))
    resp = urllib.request.urlopen(req, payload)
    if not resp or resp.status != 200:
        return 1, None
    payload = resp.read()
    body = json.loads(payload.decode('utf-8'))
    buildinfo = body['result'][1]
    return 0, buildinfo


def PollBuildResult(buildinfo):
    if len(buildinfo) != 1:
        return 1
    builder = next(iter(buildinfo))
    buildid = buildinfo[builder]
    resource = f'/builders/{builder}/builds/{buildid}'
    while True:
        time.sleep(15)
        req = urllib.request.Request(url=REMOTE_AIX_BUILDBOT_URL + resource,
                                     method='GET')
        resp = urllib.request.urlopen(req)
        if not resp or resp.status != 200:
            return 1
        result = json.loads(resp.read().decode('utf-8'))
        if len(result['builds']) != 1:
            return 1
        if result['builds'][0]['complete']:
            return result['builds'][0]['results']


if __name__ == '__main__':
    rc, buildinfo = RequestBuild()
    if rc != 0:
        sys.exit(rc)
    sys.exit(PollBuildResult(buildinfo))
