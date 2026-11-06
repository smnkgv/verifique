# A clean Linux machine, to run the free tiers from anywhere:
#   docker build -f e2e/Dockerfile -t verifique-e2e . && docker run --rm verifique-e2e
FROM rust:1-bookworm
RUN apt-get update -qq && apt-get install -y -qq python3 git
WORKDIR /verifique
COPY . .
RUN git init -q && git add -A && git -c user.name=e2e -c user.email=e2e@example.test commit -qm "This build"
RUN sh -c 'sed "/mongod --fork/,\$d" e2e/services.sh > /tmp/packages.sh && sh /tmp/packages.sh'
CMD ["sh", "-c", "sh e2e/services.sh && sh e2e/run.sh"]
