# A clean Linux machine, to run the free tiers from anywhere:
#   docker build -f e2e/Dockerfile -t verifique-e2e . && docker run --rm verifique-e2e
# Ubuntu, because MongoDB has no Debian packages for ARM, which is what Docker runs on a modern Mac.
FROM ubuntu:24.04
RUN apt-get update -qq && apt-get install -y -qq build-essential ca-certificates curl git python3
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH=/root/.cargo/bin:$PATH
WORKDIR /verifique
COPY . .
RUN git init -q && git add -A && git -c user.name=e2e -c user.email=e2e@example.test commit -qm "This build"
RUN sh -c 'sed "/mongod --fork/,\$d" e2e/services.sh > /tmp/packages.sh && sh /tmp/packages.sh'
CMD ["sh", "-c", "sh e2e/services.sh && sh e2e/run.sh"]
