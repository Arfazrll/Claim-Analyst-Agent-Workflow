<template>
  <div class="page nf">
    <h1>This page does not exist</h1>
    <p class="muted">The address may be mistyped, or the page was moved.</p>
    <RouterLink to="/" class="btn btn-primary">Go to the desk</RouterLink>
  </div>
</template>

<style scoped>
.nf {
  display: grid;
  gap: 1rem;
  justify-items: start;
  padding-top: 18vh;
  margin: 0 auto;
}
</style>
