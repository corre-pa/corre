FROM nginx:alpine

COPY nginx.conf /etc/nginx/nginx.conf
COPY site/ /usr/share/nginx/html/mcp/

EXPOSE 5580

CMD ["nginx", "-g", "daemon off;"]
