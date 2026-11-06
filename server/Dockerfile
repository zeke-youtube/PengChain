FROM node:22-bookworm-slim AS build
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 make g++ \
    && rm -rf /var/lib/apt/lists/*

ENV npm_config_jobs=1

COPY package*.json ./
RUN npm ci
COPY tsconfig.json ./
COPY src ./src
COPY migrations ./migrations
COPY public ./public
RUN npm run build

RUN npm prune --omit=dev

FROM node:22-bookworm-slim
WORKDIR /app
ENV NODE_ENV=production PENGCHAIN_DB=/data/pengchain.sqlite npm_config_jobs=1
RUN useradd --create-home appuser
COPY --from=build --chown=appuser:appuser /app/node_modules ./node_modules
COPY --from=build --chown=appuser:appuser /app/package*.json ./
COPY --from=build --chown=appuser:appuser /app/dist ./dist
COPY --from=build --chown=appuser:appuser /app/migrations ./migrations
COPY --from=build --chown=appuser:appuser /app/public ./public
RUN mkdir /data && chown appuser:appuser /data
USER appuser
EXPOSE 3000
HEALTHCHECK CMD node -e "fetch('http://127.0.0.1:3000/health').then(r=>{if(!r.ok)process.exit(1)})"
CMD ["node","dist/index.js"]
