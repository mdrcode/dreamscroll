docker run -d \
--name fake-gcs \
-p 4443:4443 \
-v $PWD/localdev/gcloud_storage/:/data \
fsouza/fake-gcs-server \
-scheme http
