/* Test-only native interposition: pause after the helper's first successful
 * unlock. No production failpoint or real backend is involved. */
#include <sys/file.h>
#include <fcntl.h>
#include <signal.h>
#include <stdlib.h>
#include <unistd.h>
#ifndef __APPLE__
#include <dlfcn.h>
#endif
static int paused;
static void pause_first_unlock(int result, int operation) {
  if (result == 0 && operation == LOCK_UN && !paused) {
    paused = 1;
    const char *path = getenv("AGEND_PROBE_UNLOCK_PATH");
    if (path) {
      int fd = open(path, O_WRONLY | O_CREAT | O_EXCL, 0600);
      if (fd >= 0) { write(fd, "paused", 6); close(fd); }
      raise(SIGSTOP);
    }
  }
}
#ifdef __APPLE__
static int probe_flock(int fd, int operation) {
  int result = flock(fd, operation);
  pause_first_unlock(result, operation);
  return result;
}
__attribute__((used)) static struct { const void *replacement; const void *original; }
interposers[] __attribute__((section("__DATA,__interpose"))) = {{probe_flock, flock}};
#else
int flock(int fd, int operation) {
  int (*original)(int, int) = dlsym(RTLD_NEXT, "flock");
  if (!original) _exit(91);
  int result = original(fd, operation);
  pause_first_unlock(result, operation);
  return result;
}
#endif
