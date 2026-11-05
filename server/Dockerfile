FROM node:22-bookworm-slim AS build
WORKDIR /app
COPY package*.json ./
RUN npm ci
COPY tsconfig.json ./
COPY src ./src
COPY migrations ./migrations
RUN npm run build
FROM node:22-bookworm-slim
WORKDIR /app
ENV NODE_ENV=production PENGCHAIN_DB=/data/pengchain.sqlite
COPY --from=build /app/package*.json ./
RUN npm ci --omit=dev && useradd --create-home appuser
COPY --from=build /app/dist ./dist
COPY --from=build /app/migrations ./migrations
RUN mkdir /data && chown appuser:appuser /data
USER appuser
EXPOSE 3000
HEALTHCHECK CMD node -e "fetch('http://127.0.0.1:3000/health').then(r=>{if(!r.ok)process.exit(1)})"
CMD ["node","dist/index.js"]
