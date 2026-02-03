docker run -d \
--name fake-gcs \
-p 4443:4443 \
-v $PWD/localdev/gcloud_storage/:/storage \
fsouza/fake-gcs-server \
-scheme http \
-data /storage

curl -X POST -H "Content-Type: application/json" \
http://localhost:4443/storage/v1/b \
-d '{"name": "dreamscroll-test1"}'


# NOTE, if you get "Not Found" errors when attempting to upload to emulator
# You probably need to create the bucket manually, since the emulator
# apparently does not (always?) automatically create the bucket for you:
#
# curl -X POST -H "Content-Type: application/json" \
#   http://localhost:4443/storage/v1/b \
#   -d '{"name": "dreamscroll-test1"}'
#
# You can list current buckets with:
# curl http://localhost:4443/storage/v1/b
