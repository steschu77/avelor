// The jobs panel (#jobs): a card per job the host runs, from its `job`
// events (api.md#events), removed a short while after it ends.
// A job that can be stopped has a Stop button while it runs.

import { $, accEl } from '../core/dom.js';
import { rpcOn } from '../core/rpc.js';
import { rpcCall } from '../core/rpc.js';
import { iconEl } from '../ui/icons.js';

const jobCards = new Map(); // job id -> card element
const JOB_REMOVE_DELAY_MS = 1500;
const JOB_NOTE_DELAY_MS = 8000;
const JOB_STOP_ICON = '#i-stop';

// Show `job` ({ id, label, state, done, total, note, stoppable }): its card,
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
  const note = job.state === 'stopping' ? 'stopping\u2026' : job.note;
  el.querySelector('.job__details').textContent = note
    ? `${job.done} / ${job.total} \u00B7 ${note}` : `${job.done} / ${job.total}`;
  renderJobStop(el, job);

  if (job.state === 'done') {
    jobCards.delete(job.id);
    el.classList.add('is-done');
    setTimeout(() => el.remove(), job.note ? JOB_NOTE_DELAY_MS : JOB_REMOVE_DELAY_MS);
  }
}

// The card's Stop button, while the job can be stopped.
function renderJobStop(el, job) {
  const head = el.querySelector('.job__head');
  let stop = head.querySelector('button');
  if (!job.stoppable) {
    stop?.remove();
    return;
  }
  if (stop) return;
  stop = accEl('button', 'icon-btn icon-btn--quiet push');
  stop.append(iconEl(JOB_STOP_ICON));
  stop.type = 'button';
  stop.title = 'Stop';
  stop.addEventListener('click', () => {
    stop.setAttribute('aria-disabled', 'true');
    rpcCall('cancel_job', { id: job.id });
  });
  head.append(stop);
}

rpcOn('job', renderJob);
