export const aiConfig = $state({
  provider: 'openai' as 'openai' | 'anthropic' | 'ollama',
  endpoint: '',
  model: 'gpt-4o',
  maxTokens: 4096,
  maxTurns: 50,
  rowLimit: 500,
  sendResultsToAi: true,
  enableBlastRadiusCheck: true,
});
