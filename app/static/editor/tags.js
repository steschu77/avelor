// The editor's DOM vocabulary (editor.md#block-model, markdown.md#dom-contract):
// the blocks that sit at the root, and every tag the content may hold, which
// the paste keeps and the serializer writes.

export const HEADING_TAGS = ['H1', 'H2', 'H3', 'H4', 'H5', 'H6'];

export const ROOT_BLOCK_TAGS = ['P', ...HEADING_TAGS, 'UL', 'OL', 'BLOCKQUOTE', 'PRE', 'HR', 'TABLE'];

export const CONTENT_TAGS = [
  ...ROOT_BLOCK_TAGS, 'LI', 'THEAD', 'TBODY', 'TR', 'TH', 'TD', 'STRONG', 'EM', 'DEL', 'CODE', 'A', 'IMG', 'BR',
];
