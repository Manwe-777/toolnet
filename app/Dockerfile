# The owner app (static PWA), built and served by an unprivileged nginx.
FROM node:22-alpine AS build
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci && npm cache clean --force
COPY index.html vite.config.js ./
COPY public public
COPY src src
RUN npm run build

FROM nginxinc/nginx-unprivileged:1.27-alpine
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/dist /usr/share/nginx/html
EXPOSE 8080
HEALTHCHECK --interval=60s --timeout=5s CMD wget -qO /dev/null http://127.0.0.1:8080/ || exit 1
