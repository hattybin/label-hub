# Tailscale ACL policy for the Label Hub fleet

Apply this in the Tailscale admin console (Access controls). It enforces:
- the **control plane** can reach each node's management API (port 8081) and the
  node's print webhook (8080, for remote test-prints);
- **nodes cannot talk to each other**;
- **admins** can reach node consoles and the control dashboard.

```jsonc
{
  "tagOwners": {
    "tag:lh-node":    ["autogroup:admin"],
    "tag:lh-control": ["autogroup:admin"]
  },

  "groups": {
    "group:lh-admins": ["you@example.com"]
  },

  "acls": [
    // Control plane → node management API + webhook
    {
      "action": "accept",
      "src": ["tag:lh-control"],
      "dst": ["tag:lh-node:8080", "tag:lh-node:8081"]
    },
    // Nodes → control plane node API (enroll/heartbeat/config/events)
    {
      "action": "accept",
      "src": ["tag:lh-node"],
      "dst": ["tag:lh-control:9090"]
    },
    // Admins → node consoles + control dashboard
    {
      "action": "accept",
      "src": ["group:lh-admins"],
      "dst": ["tag:lh-node:8081", "tag:lh-control:9091", "tag:lh-control:9090"]
    }
    // (No rule lets tag:lh-node reach tag:lh-node — nodes are isolated from each other.)
  ],

  // Auth keys minted by the control plane's OAuth client are tagged tag:lh-node.
  "tagOwners": {
    "tag:lh-node": ["autogroup:admin", "tag:lh-control"]
  }
}
```

Notes:
- The control plane host joins the tailnet as `tag:lh-control` (auth key or OAuth).
- Nodes join as `tag:lh-node` (key minted at enrollment, or baked into the image
  config). Tagged devices don't expire, so they stay connected unattended.
- MagicDNS gives each node a stable name; the control plane reaches nodes at
  `http://<node-hostname>:8081`.
