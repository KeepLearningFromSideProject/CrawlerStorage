TAG='dansnow/crawler-storage'
id="$(docker run -d --rm \
           --device /dev/fuse \
           --cap-add SYS_ADMIN \
           --security-opt apparmor:unconfined \
           -p 127.0.0.1:5050:5000 \
           "$TAG" bash /entrypoint.sh)"

trap 'docker kill "$id" > /dev/null' EXIT
echo "wait service up"
sleep 5

if ! poetry run pytest; then
  docker logs "$id"
  exit 1
fi
