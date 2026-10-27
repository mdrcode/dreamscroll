# Canonical projection used by gcloud/prod-apply.sh to compare a checked-in
# manifest with the live Cloud Run resource.
#
# - deletes server-managed and client-marker fields (delpaths tolerates paths
#   whose parents are absent, e.g. a manifest with no metadata.labels),
# - drops the container image, which prod-apply.sh injects at apply time,
# - sorts env arrays by name so ordering cannot fake drift,
# - emits one "path = value" line per scalar, which diffs as named fields.
#
# Runs under jq and jaq 3.x. Do not use `.[]?` inside del/update paths here:
# jaq 3.x errors on a missing parent instead of treating it as a no-op.

def normalize_container:
  del(.image)
  | .env |= (if type == "array" then sort_by(.name) else . end);

def server_paths:
    ["status"]
  , ["metadata", "namespace"]
  , ["metadata", "uid"]
  , ["metadata", "selfLink"]
  , ["metadata", "resourceVersion"]
  , ["metadata", "generation"]
  , ["metadata", "creationTimestamp"]
  , ["metadata", "labels", "cloud.googleapis.com/location"]
  , ["metadata", "annotations", "run.googleapis.com/urls"]
  , ["metadata", "annotations", "run.googleapis.com/ingress-status"]
  , ["metadata", "annotations", "run.googleapis.com/operation-id"]
  , ["metadata", "annotations", "run.googleapis.com/client-name"]
  , ["metadata", "annotations", "serving.knative.dev/creator"]
  , ["metadata", "annotations", "serving.knative.dev/lastModifier"]
  , ["spec", "template", "metadata", "labels", "client.knative.dev/nonce"]
  , ["spec", "template", "metadata", "annotations", "run.googleapis.com/client-name"]
  , ["metadata", "annotations", "run.googleapis.com/client-version"]
  , ["metadata", "annotations", "run.googleapis.com/creator"]
  , ["metadata", "annotations", "run.googleapis.com/lastModifier"]
  , ["metadata", "labels", "run.googleapis.com/lastUpdatedTime"]
  , ["spec", "template", "metadata", "annotations", "run.googleapis.com/client-version"]
  , ["spec", "template", "spec", "serviceAccountName"]
  , ["spec", "template", "spec", "template", "spec", "serviceAccountName"];

def delp($p):
  . as $doc
  | (try ($doc | getpath($p)) catch null) as $v
  | if $v == null then $doc else ($doc | delpaths([$p])) end;

reduce (server_paths) as $p (.; delp($p))
| (if (.spec.template.spec.containers | type) == "array"
   then .spec.template.spec.containers |= map(normalize_container)
   else . end)
| (if (.spec.template.spec.template.spec.containers | type) == "array"
   then .spec.template.spec.template.spec.containers |= map(normalize_container)
   else . end)
| [paths(scalars) as $p | "\($p | map(tostring) | join(".")) = \(getpath($p))"]
| .[]
