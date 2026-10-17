// The models features pick: the AI providers.

export const AI_PROVIDERS = [
  { kind: 'anthropic', name: 'Anthropic', placeholder: 'sk-ant-\u2026' },
  { kind: 'openai', name: 'OpenAI', placeholder: 'sk-\u2026' },
  { kind: 'mistral', name: 'Mistral', placeholder: 'API key' },
  { kind: 'gemini', name: 'Gemini', placeholder: 'AIza\u2026' },
  { kind: 'ollama', name: 'Ollama', local: true, address: 'http://localhost:11434' },
];

export function aiProvider(kind) {
  return AI_PROVIDERS.find(p => p.kind === kind);
}

// ----------------------------------------------------------------------------
// formatting

export function aiHost(address) {
  return (address ?? '').replace(/^[a-z]+:\/\//i, '').split('/')[0];
}

function aiIsLoopback(address) {
  return /^(localhost|127\.|\[::1\])/i.test(aiHost(address));
}

// The provider's name; Ollama's with "local", or with its host elsewhere.
export function aiTitle(kind, account) {
  const provider = aiProvider(kind);
  if (!provider.local || !account) return provider.name;
  return `${provider.name} \u00b7 ${aiIsLoopback(account.address) ? 'local' : aiHost(account.address)}`;
}

// A price or limit: "$10", "$1.25", "$0.40", "$0.075".
export function aiUsd(n) {
  if (Number.isInteger(n)) return `$${n}`;
  const cents = Math.round(n * 1e5) / 1e3;
  return `$${Number.isInteger(cents) ? n.toFixed(2) : n.toFixed(3)}`;
}
