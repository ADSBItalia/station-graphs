FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends rrdtool ca-certificates tzdata && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY adsbitalia-station-graphs /app/adsbitalia-station-graphs
COPY web /app/web
ENV PORT=8085
EXPOSE 8085
CMD ["/app/adsbitalia-station-graphs"]
