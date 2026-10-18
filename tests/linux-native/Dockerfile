ARG BASE_IMAGE
FROM ${BASE_IMAGE}
ARG FULL_DESKTOP=0
COPY install-runtime.sh /tmp/install-runtime.sh
RUN sh /tmp/install-runtime.sh "$FULL_DESKTOP"
COPY . /driver
ARG COMPATIBILITY_DRIVER=0
RUN if [ "$COMPATIBILITY_DRIVER" = 1 ]; then sh /driver/build-qa-mesa.sh; fi
RUN useradd --create-home --uid 1001 chda-test
ENTRYPOINT ["sh", "/driver/run-package.sh"]
