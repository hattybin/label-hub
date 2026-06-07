# Exposing label-hub with Azure Relay Hybrid Connections (azbridge)

This is the Azure-native equivalent of a tunnel and the recommended option for
work/production. The Pi runs **azbridge** (the Azure Relay Bridge), which keeps an
outbound-only connection to an Azure Relay namespace — no inbound firewall changes.
D365 calls the Relay's public HTTPS endpoint, which is forwarded to the hub on
localhost.

## 1. Create the Azure resources (once)

```bash
# Relay namespace
az relay namespace create -g <rg> -n <namespace> -l <region>

# A Hybrid Connection (HC) for this site, with client auth required
az relay hyco create -g <rg> --namespace-name <namespace> -n labelhub-plant1 \
  --requires-client-authorization true

# A SAS rule the Pi will use to listen (Listen+Send)
az relay hyco authorization-rule create -g <rg> --namespace-name <namespace> \
  --hybrid-connection-name labelhub-plant1 -n bridge --rights Listen Send

# Grab the connection string for that rule
az relay hyco authorization-rule keys list -g <rg> --namespace-name <namespace> \
  --hybrid-connection-name labelhub-plant1 -n bridge --query primaryConnectionString -o tsv
```

## 2. Install azbridge on the Pi

Download the linux-arm64 build from
<https://github.com/Azure/azure-relay-bridge/releases> and unpack it, e.g. to
`/opt/azbridge`.

## 3. Run azbridge as a remote forwarder

`-H` (remote forward) binds the Hybrid Connection to a local HTTP endpoint:

```bash
export AZURE_BRIDGE_CONNECTIONSTRING="<the connection string from step 1>"
/opt/azbridge/azbridge -H labelhub-plant1:http/localhost:8080
```

This makes requests arriving at the Relay's HTTPS endpoint flow to
`http://localhost:8080` (label-hub's `PORT`).

Run it as a systemd service alongside `label-hub.service` so it starts on boot.
A minimal unit:

```ini
[Unit]
Description=azbridge for label-hub
After=network-online.target
Wants=network-online.target

[Service]
Environment=AZURE_BRIDGE_CONNECTIONSTRING=Endpoint=sb://...;SharedAccessKeyName=bridge;SharedAccessKey=...
ExecStart=/opt/azbridge/azbridge -H labelhub-plant1:http/localhost:8080
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

## 4. In D365

Set the External Service **instance** Base URL to the Relay HTTPS endpoint:

```
https://<namespace>.servicebus.windows.net/labelhub-plant1
```

so the inbound endpoint is
`https://<namespace>.servicebus.windows.net/labelhub-plant1/api/print/inbound`.

D365 must also present a valid Relay **SAS token** to reach the endpoint (Relay
requires client authorization). Configure that on the D365 side per your Relay
SAS `Send` rule, in addition to the hub's own `INBOUND_SECRET` (the two layers are
independent: Relay authorizes reaching the bridge; `INBOUND_SECRET` authorizes the
print itself).

## Fallback: Service Bus queue

If HTTP-over-Relay proves awkward in your environment, the more decoupled pattern
is an Azure Function (public) that drops jobs on a Service Bus queue which the hub
polls. That survives the Pi being offline and adds automatic retries, at the cost
of more moving parts. Not included in the initial build — open an issue if you want
the polling consumer added.
