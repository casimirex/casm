# Check an architecture against real infrastructure

Every other check asks whether an architecture is internally coherent. This one asks
whether it is *true*.

## From Terraform state

```console
$ casm drift --inventory terraform.tfstate --from terraform
architecture.yaml: 6 node(s) declared

~ node 'orders-db' (database) is declared but was not found in the inventory
~ resource 'aws_s3_bucket.audit-logs' (storage) exists but is not declared

2 drift(s) against terraform: 5 node(s) matched
```

Only `managed` resources are considered — a `data` block describes something Terraform
reads rather than owns, so its absence is not drift.

## Binding a node to a resource

Matching on names alone fails immediately in practice: a node called `orders-db` is an
`aws_db_instance` called `primary`. Declare the binding:

```yaml
- name: orders-db
  type: database
  metadata:
    infrastructure-id: aws_db_instance.primary
```

Name equality remains as a fallback for the easy case. **An explicit binding wins**: if it
names a resource that no longer exists, that is reported as missing even when a
same-named resource happens to be present. A declared binding is a statement of intent,
and silently falling back would hide that the thing it named is gone.

## What is exempt

External systems and humans. `partner-bank` is not in your Terraform state and never will
be, so expecting it there would report drift on every run.

## Type mismatches

If a bound resource's type is one CASIMIR recognises and it disagrees with the node's
declared type, that is reported. If the type is *unrecognised*, nothing is asserted —
inventing a disagreement from a resource type CASIMIR has not been taught is worse than
staying quiet.

The recognised set covers the common AWS, GCP, and Azure resources. Anything else maps to
"unknown", which is honest rather than wrong.

## Your own inventory

Not using Terraform? The native format is small:

```json
{
  "source": "our-cmdb",
  "resources": [
    { "id": "svc-orders", "name": "orders", "node-type": "service" },
    { "id": "rds-primary", "name": "orders-db", "node-type": "database" }
  ]
}
```

```console
$ casm drift --inventory inventory.json --from native
```

`node-type` is optional. Omit it and CASIMIR checks existence without asserting anything
about types.

## In CI

```console
$ casm drift --inventory terraform.tfstate --from terraform --fail-on-drift
```

Start advisory. Infrastructure drifts for legitimate reasons, and you want to see the
shape of the noise before you gate on it.
