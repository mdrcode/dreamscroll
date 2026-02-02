docker run -d \
--name fake-gcs \
-p 4443:4443 \
-e "GCS_SERVER_PORT=4443" \
-e "GCS_SERVER_HOST=0.0.0.0" \
-v $PWD/localdev/gcloud_storage/:/data \
fsouza/fake-gcs-server \
-- \
-scheme http
