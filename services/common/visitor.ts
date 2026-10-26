/// What the visitor MAC covers before the visitor's name, so the signature
/// means "this Worker names this visitor" and nothing else (ov-811). The site's
/// and the web client's Workers sign with it (`visitorHeaders` in relay.ts) and
/// the relay checks it (`visitorKey` in services/relay/src/index.ts): one
/// string, here, so the two halves can't disagree.
export const VISITOR_CONTEXT = 'farcooler visitor v1\n'
