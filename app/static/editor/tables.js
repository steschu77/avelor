// Tables (editor.md#tables): cells, rows and columns, a table inserted and its
// parts deleted, Tab and Enter in a cell.

export function createTables(editor, {
  CONVERTIBLE_TAGS, emptyParagraph, getBlockNode, getRootBlock, getSelectionRange, isEmptyBlock,
  placeCaretAtStart, selectRange,
}) {
  // ---------------------------------------------------------------------------
  // Tables -- see editor.md#tables for the shape and the editing model.
  function getEnclosingCell(node) {
    if (!node) return null;
    if (node.nodeType === Node.TEXT_NODE) node = node.parentNode;
    const cell = node.closest && node.closest('td, th');
    return cell && editor.contains(cell) ? cell : null;
  }

  function tableRows(table) {
    return Array.from(table.querySelectorAll(':scope > thead > tr, :scope > tbody > tr'));
  }

  function tableCells(table) {
    return tableRows(table).flatMap(row => Array.from(row.children));
  }

  function cellIndex(cell) {
    return Array.prototype.indexOf.call(cell.parentNode.children, cell);
  }

  function isHeaderRow(row) {
    return row.parentNode.tagName === 'THEAD';
  }

  function createCell(tagName) {
    const cell = document.createElement(tagName);
    cell.appendChild(document.createElement('br'));
    return cell;
  }

  function createRow(cols, cellTag) {
    const tr = document.createElement('tr');
    for (let c = 0; c < cols; c++) tr.appendChild(createCell(cellTag));
    return tr;
  }

  // Select a cell's content so typing replaces it (spreadsheet-style).
  function moveToCell(cell) {
    const r = document.createRange();
    r.selectNodeContents(cell);
    selectRange(r);
  }

  // The block after `el`, creating a paragraph if there is none or the next
  // block can't hold a caret (rule, table).
  function blockAfter(el) {
    let next = el.nextSibling;
    if (!next || !CONVERTIBLE_TAGS.includes(next.tagName)) {
      // (the trailing-paragraph invariant makes this rare at document end)
      const p = emptyParagraph();
      el.parentNode.insertBefore(p, next);
      next = p;
    }
    return next;
  }

  function insertTable(cols = 2, bodyRows = 2) {
    const table = document.createElement('table');
    const thead = document.createElement('thead');
    thead.appendChild(createRow(cols, 'th'));
    const tbody = document.createElement('tbody');
    for (let r = 0; r < bodyRows; r++) tbody.appendChild(createRow(cols, 'td'));
    table.appendChild(thead);
    table.appendChild(tbody);

    const range = getSelectionRange();
    const block = range ? getBlockNode(range.startContainer) : null;
    const root = range ? getRootBlock(range.startContainer) : null;
    if (block && block.parentNode === editor && block.tagName === 'P' && isEmptyBlock(block)) {
      block.replaceWith(table);
    } else if (root) {
      editor.insertBefore(table, root.nextSibling);
    } else {
      editor.appendChild(table);
    }
    blockAfter(table);
    moveToCell(thead.firstChild.firstChild);
  }

  function insertRowBelow(cell) {
    const row = cell.parentNode;
    const table = row.closest('table');
    const tr = createRow(row.children.length, 'td');
    if (isHeaderRow(row)) {
      let tbody = table.querySelector(':scope > tbody');
      if (!tbody) {
        tbody = document.createElement('tbody');
        table.appendChild(tbody);
      }
      tbody.insertBefore(tr, tbody.firstChild);
    } else {
      row.parentNode.insertBefore(tr, row.nextSibling);
    }
    moveToCell(tr.children[Math.min(cellIndex(cell), tr.children.length - 1)]);
  }

  // The header row can't be deleted (markdown needs one); delete the table.
  function deleteRow(cell) {
    const row = cell.parentNode;
    if (isHeaderRow(row)) return;
    const table = row.closest('table');
    const rows = tableRows(table);
    const idx = rows.indexOf(row);
    const col = cellIndex(cell);
    row.remove();
    const target = rows[idx + 1] || rows[idx - 1];
    moveToCell(target.children[Math.min(col, target.children.length - 1)]);
  }

  function insertColumnRight(cell) {
    const idx = cellIndex(cell);
    const table = cell.closest('table');
    let target = null;
    for (const row of tableRows(table)) {
      const ref = row.children[idx];
      const fresh = createCell(isHeaderRow(row) ? 'th' : 'td');
      row.insertBefore(fresh, ref ? ref.nextSibling : null);
      if (row === cell.parentNode) target = fresh;
    }
    moveToCell(target);
  }

  function deleteColumn(cell) {
    const table = cell.closest('table');
    const row = cell.parentNode;
    if (row.children.length <= 1) {
      deleteTable(table);
      return;
    }
    const idx = cellIndex(cell);
    for (const r of tableRows(table)) r.children[idx]?.remove();
    moveToCell(row.children[Math.min(idx, row.children.length - 1)]);
  }

  function deleteTable(table) {
    const next = blockAfter(table);
    table.remove();
    placeCaretAtStart(next);
  }

  // Tab: next cell, appending a row after the last one. Shift+Tab: previous.
  function tabInTable(cell, backwards) {
    const cells = tableCells(cell.closest('table'));
    const i = cells.indexOf(cell);
    if (backwards) {
      if (i > 0) moveToCell(cells[i - 1]);
    } else if (i < cells.length - 1) {
      moveToCell(cells[i + 1]);
    } else {
      insertRowBelow(cell);
    }
  }

  // Enter: the cell below; from the last row, the block after the table.
  function enterInTable(cell) {
    const row = cell.parentNode;
    const table = row.closest('table');
    const rows = tableRows(table);
    const below = rows[rows.indexOf(row) + 1];
    if (below) {
      moveToCell(below.children[Math.min(cellIndex(cell), below.children.length - 1)]);
    } else {
      placeCaretAtStart(blockAfter(table));
    }
  }

  return {
    deleteColumn, deleteRow, deleteTable, enterInTable, getEnclosingCell, insertColumnRight, insertRowBelow,
    insertTable, tabInTable,
  };
}
