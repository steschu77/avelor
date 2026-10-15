// The page's entry (architecture.md#page): it tells the host the page
// listens (`ready`). Nothing imports it.

import { rpcCall } from './core/rpc.js';

// The page listens.
rpcCall('ready');
