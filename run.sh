TAG='dansnow/crawler-storage'
docker run -it --rm \
           --device /dev/fuse \
           --cap-add SYS_ADMIN \
           --volume "$(realpath storage):/storage" \
           --security-opt apparmor:unconfined \
      "$TAG" bash /entrypoint.sh
