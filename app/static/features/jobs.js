// The jobs panel (#jobs): a card per job the host runs, from its `job`
// events (api.md#events), removed a short while after it ends.

import { $, accEl } from '../core/dom.js';
import { rpcOn } from '../core/rpc.js';

const jobCards = new Map(); // job id -> card element
const JOB_REMOVE_DELAY_MS = 1500;
const JOB_NOTE_DELAY_MS = 8000;

// Show `job` ({ id, label, state, done, total, note }): its card,
// made on the first event. `note` is shown after the count, e.g. an ended
// job's summary; a card with a note stays a while longer, so it can be read.
function renderJob(job) {
  let el = jobCards.get(job.id);
  if (!el) {
    el = accEl('div', 'job');
    el.innerHTML = `
      <div class="job__head"><span class="job__name"></span></div>
      <div class="progress"><i style="width:0%"></i></div>
      <div class="job__details label"></div>`;
    $('jobs').appendChild(el);
    jobCards.set(job.id, el);
  }
  el.querySelector('.job__name').textContent = job.label;
  const pct = job.total > 0 ? Math.round((job.done / job.total) * 100) : 100;
  el.querySelector('.progress > i').style.width = `${pct}%`;
  el.querySelector('.job__details').textContent = job.note
    ? `${job.done} / ${job.total} \u00B7 ${job.note}` : `${job.done} / ${job.total}`;

  if (job.state === 'done') {
    jobCards.delete(job.id);
    el.classList.add('is-done');
    setTimeout(() => el.remove(), job.note ? JOB_NOTE_DELAY_MS : JOB_REMOVE_DELAY_MS);
  }
}

rpcOn('job', renderJob);
