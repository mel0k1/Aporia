// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <sys/mman.h>
#include <sys/resource.h>
#include <string.h>
#include <unistd.h>

#include "../../common/test.h"

#define PAGE_SIZE 4096

FN_TEST(mlock_err)
{
	char *addr = TEST_SUCC(mmap(NULL, 2 * PAGE_SIZE, PROT_READ | PROT_WRITE,
				    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));

	// The address must be page-aligned.
	TEST_ERRNO(mlock(addr + 1, PAGE_SIZE), EINVAL);
	TEST_ERRNO(munlock(addr + 1, PAGE_SIZE), EINVAL);
	// The range must be within the user address space.
	TEST_ERRNO(mlock((void *)-PAGE_SIZE, PAGE_SIZE), EINVAL);
	TEST_ERRNO(mlock(addr, (size_t)-2 * PAGE_SIZE), EINVAL);

	// Unlocking an unlocked range is not an error.
	TEST_SUCC(munlock(addr, 2 * PAGE_SIZE));

	// Create an unmapped hole in the middle of the range.
	TEST_SUCC(mmap(addr + 2 * PAGE_SIZE, PAGE_SIZE, PROT_READ | PROT_WRITE,
		       MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0));
	TEST_ERRNO(mlock(addr, 3 * PAGE_SIZE), ENOMEM);
	TEST_ERRNO(munlock(addr, 3 * PAGE_SIZE), ENOMEM);

	TEST_SUCC(munmap(addr, 3 * PAGE_SIZE));
}
END_TEST()

FN_TEST(mlock_unlock_roundtrip)
{
	char *addr = TEST_SUCC(mmap(NULL, 2 * PAGE_SIZE, PROT_READ | PROT_WRITE,
				    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));

	// mlock populates the pages, so accesses must not fail afterwards.
	TEST_SUCC(mlock(addr, 2 * PAGE_SIZE));
	memset(addr, 'A', 2 * PAGE_SIZE);
	TEST_RES(memcmp(addr, "AAAA", 4), _ret == 0);

	// Locking an already locked range is not an error.
	TEST_SUCC(mlock(addr, PAGE_SIZE));

	TEST_SUCC(munlock(addr, 2 * PAGE_SIZE));
	// Unlocking an unlocked range is not an error.
	TEST_SUCC(munlock(addr, PAGE_SIZE));

	TEST_SUCC(munmap(addr, 2 * PAGE_SIZE));
}
END_TEST()

FN_TEST(mlock2_flags)
{
	char *addr = TEST_SUCC(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE,
				    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));

	TEST_ERRNO(mlock2(addr, PAGE_SIZE, 0x2), EINVAL);
	TEST_ERRNO(mlock2(addr, PAGE_SIZE, -1), EINVAL);

	TEST_SUCC(mlock2(addr, PAGE_SIZE, MLOCK_ONFAULT));
	memset(addr, 'A', PAGE_SIZE);
	TEST_RES(memcmp(addr, "AAAA", 4), _ret == 0);
	TEST_SUCC(munlock(addr, PAGE_SIZE));

	TEST_SUCC(mlock2(addr, PAGE_SIZE, 0));
	TEST_SUCC(munlock(addr, PAGE_SIZE));

	TEST_SUCC(munmap(addr, PAGE_SIZE));
}
END_TEST()

FN_TEST(mlockall_err)
{
	// The flags must not be zero or contain unknown bits.
	TEST_ERRNO(mlockall(0), EINVAL);
	TEST_ERRNO(mlockall(0x8), EINVAL);
	// MCL_ONFAULT must be combined with MCL_CURRENT or MCL_FUTURE.
	TEST_ERRNO(mlockall(MCL_ONFAULT), EINVAL);
}

END_TEST()

FN_TEST(mlockall_future_locks_new_mappings)
{
	TEST_SUCC(mlockall(MCL_FUTURE));

	char *addr = TEST_SUCC(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE,
				    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));
	memset(addr, 'B', PAGE_SIZE);
	TEST_RES(memcmp(addr, "BBBB", 4), _ret == 0);

	TEST_SUCC(munlockall());
	TEST_SUCC(munmap(addr, PAGE_SIZE));
}
END_TEST()

FN_TEST(mlockall_current)
{
	struct rlimit old_rlimit;
	TEST_SUCC(getrlimit(RLIMIT_MEMLOCK, &old_rlimit));

	// The whole address space of the test process exceeds the default
	// memory lock limit, so raise the limit first.
	struct rlimit unlimited_rlimit = {
		.rlim_cur = RLIM_INFINITY,
		.rlim_max = RLIM_INFINITY,
	};
	TEST_SUCC(setrlimit(RLIMIT_MEMLOCK, &unlimited_rlimit));

	TEST_SUCC(mlockall(MCL_CURRENT));
	// Locking all current mappings twice is not an error.
	TEST_SUCC(mlockall(MCL_CURRENT));
	TEST_SUCC(munlockall());
	// munlockall without any locked mappings is not an error.
	TEST_SUCC(munlockall());

	TEST_SUCC(setrlimit(RLIMIT_MEMLOCK, &old_rlimit));
}
END_TEST()

FN_TEST(mlock_rlimit_memlock)
{
	struct rlimit old_rlimit;
	TEST_SUCC(getrlimit(RLIMIT_MEMLOCK, &old_rlimit));

	char *addr = TEST_SUCC(mmap(NULL, 2 * PAGE_SIZE, PROT_READ | PROT_WRITE,
				    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));

	// Allow locking a single page.
	struct rlimit one_page_rlimit = {
		.rlim_cur = PAGE_SIZE,
		.rlim_max = old_rlimit.rlim_max,
	};
	TEST_SUCC(setrlimit(RLIMIT_MEMLOCK, &one_page_rlimit));

	TEST_SUCC(mlock(addr, PAGE_SIZE));
	// The limit is reached, locking more pages must fail.
	TEST_ERRNO(mlock(addr + PAGE_SIZE, PAGE_SIZE), ENOMEM);

	TEST_SUCC(munlock(addr, PAGE_SIZE));
	// The limit is reached, locking two pages at once must fail.
	TEST_ERRNO(mlock(addr, 2 * PAGE_SIZE), ENOMEM);

	// Restore the original limit.
	TEST_SUCC(setrlimit(RLIMIT_MEMLOCK, &old_rlimit));

	// With the original limit restored, locking two pages must succeed.
	TEST_SUCC(mlock(addr, 2 * PAGE_SIZE));
	TEST_SUCC(munlock(addr, 2 * PAGE_SIZE));

	TEST_SUCC(munmap(addr, 2 * PAGE_SIZE));
}
END_TEST()
