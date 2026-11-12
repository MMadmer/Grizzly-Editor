package main

import (
	"fmt"
	"os"
	"regexp"
	"strconv"
	"strings"
)

const (
	programName = "github_mock"
	description = "Local mock of the GitHub and VirusTotal endpoints the editor uses to publish a mod release, " +
		"submit it to the Mod Browser and check its packages."
	defaultUploadResults = `{"EngineA": "undetected", "EngineB": "undetected", "EngineC": "harmless"}`
)

type config struct {
	port                                                int
	token, repo                                         string
	private, archived, noPush, empty                    bool
	seed                                                string
	dropUpload                                          int
	renameAssets, protectTags, latestOnEdit, noDigest   bool
	corruptDigest                                       string
	staleLatest, throttleKbps                           int
	clientID                                            string
	deviceApproveAfter                                  int
	deviceDeny                                          bool
	deviceInterval                                      int
	log, login, catalogRepo, catalogSeed, vtKey, vtSeed string
	vtQueue                                             int
	vtHideNew                                           bool
	uploadResultsText                                   string
	vtUploadResults                                     object
	vtRate, vtMaxUpload                                 int
	vtRateWindowMs                                      int
}

// option is one command line flag: target is a *string, an *int, a *bool (a switch) or nil (--help).
type option struct {
	name, meta, help string
	target           any
}

func (c *config) options() []option {
	return []option{
		{"--help", "", "show this help message and exit", nil},
		{"--port", "PORT", "listening port on 127.0.0.1; 0 picks a free one", &c.port},
		{"--token", "TOKEN", "the only accepted bearer token", &c.token},
		{"--repo", "REPO", "owner/name of the mod's own repository (releases); the flags below up to --empty apply to it", &c.repo},
		{"--private", "", "private repository: web downloads answer 404", &c.private},
		{"--archived", "", "archived repository: writes answer 403", &c.archived},
		{"--no-push", "", "no push permission: writes answer 403", &c.noPush},
		{"--empty", "", "creating a release answers 422 Repository is empty.", &c.empty},
		{"--seed", "FILE", "JSON list of releases to pre-create", &c.seed},
		{"--drop-upload", "N", "reset the N-th upload mid-body", &c.dropUpload},
		{"--rename-assets", "", "store asset names with '-' replaced by '.'", &c.renameAssets},
		{"--protect-tags", "", "deleting a tag ref answers 422, as a tag ruleset does", &c.protectTags},
		{"--latest-on-edit", "", "a PATCH without make_latest marks a published full release latest, GitHub's documented default", &c.latestOnEdit},
		{"--no-digest", "", "asset JSON carries no digest", &c.noDigest},
		{"--corrupt-digest", "SUFFIX", "report a wrong digest for matching asset names", &c.corruptDigest},
		{"--stale-latest", "N", "latest/download serves the previous latest release for N requests after a change", &c.staleLatest},
		{"--throttle-kbps", "K", "read upload bodies at K kilobits per second (1 kbps = 125 bytes/s)", &c.throttleKbps},
		{"--client-id", "CLIENT_ID", "the only accepted OAuth client id", &c.clientID},
		{"--device-approve-after", "K", "the K-th counted token poll ends the device flow", &c.deviceApproveAfter},
		{"--device-deny", "", "the device flow ends with access_denied", &c.deviceDeny},
		{"--device-interval", "S", "minimum seconds between polls", &c.deviceInterval},
		{"--log", "FILE", "append one JSON line per request", &c.log},
		{"--login", "NAME", "the account's login (GET /user; the API opens issues and writes comments as it); default: the owner part of --repo", &c.login},
		{"--catalog-repo", "O/N", "owner/name of the Mod Browser catalog repository (files and issues; default branch main); a --login that owns it is the maintainer: push permission, and every issue is theirs to edit", &c.catalogRepo},
		{"--catalog-seed", "DIR", "every file below DIR goes into the catalog's first commit", &c.catalogSeed},
		{"--vt-key", "KEY", "the only accepted VirusTotal key (x-apikey)", &c.vtKey},
		{"--vt-seed", "FILE", `JSON {"<sha256>": {"<engine>": "<category>"}}: files VirusTotal already knows`, &c.vtSeed},
		{"--vt-queue", "N", "after an upload the file's report stays queued, without verdicts, for N lookups", &c.vtQueue},
		{"--vt-hide-new", "", "while a file's first analysis is queued, GET /files answers 404, as VirusTotal does", &c.vtHideNew},
		{"--vt-upload-results", "JSON", `the verdicts an uploaded file ends up with, {"<engine>": "<category>"}; categories: ` + strings.Join(vtCategories, ", "), &c.uploadResultsText},
		{"--vt-rate", "N", "more than N VirusTotal requests within the rate window answer 429; 0 = unlimited", &c.vtRate},
		{"--vt-rate-window", "MS", "the rate window of --vt-rate in milliseconds (default 60000, VirusTotal's minute)", &c.vtRateWindowMs},
		{"--vt-max-upload", "BYTES", fmt.Sprintf("the largest body POST /files takes (413 above it); an upload URL takes up to %d", vtUploadURLLimit), &c.vtMaxUpload},
	}
}

func usage(options []option) string {
	var text strings.Builder
	text.WriteString("usage: " + programName + " [-h]")
	for _, opt := range options[1:] {
		if opt.meta == "" {
			fmt.Fprintf(&text, " [%s]", opt.name)
		} else {
			fmt.Fprintf(&text, " [%s %s]", opt.name, opt.meta)
		}
	}
	return text.String()
}

func helpText(options []option) string {
	const helpColumn = 24
	var text strings.Builder
	fmt.Fprintf(&text, "%s\n\n%s\n\noptions:\n", usage(options), description)
	for _, opt := range options {
		label := "  " + opt.name
		if opt.name == "--help" {
			label = "  -h, --help"
		} else if opt.meta != "" {
			label += " " + opt.meta
		}
		if len(label) < helpColumn-1 {
			fmt.Fprintf(&text, "%-*s%s\n", helpColumn, label, opt.help)
		} else {
			fmt.Fprintf(&text, "%s\n%*s%s\n", label, helpColumn, "", opt.help)
		}
	}
	return text.String()
}

// failUsage is argparse's parser.error: the usage and the problem on stderr, exit code 2.
// usageFailure is what failUsage raises while parseArgsChecked reads a /__restart: the process lives on.
type usageFailure struct{ problem string }

// checking is set while parseArgsChecked runs; /__restart holds the holder's lock, one at a time.
var checking bool

func failUsage(options []option, problem string) {
	if checking {
		panic(usageFailure{problem})
	}
	fmt.Fprintf(os.Stderr, "%s\n%s: error: %s\n", usage(options), programName, problem)
	os.Exit(2)
}

// parseArgsChecked reads arguments as parseArgs does, answering a bad one as an error instead of
// ending the process; --help is refused.
func parseArgsChecked(arguments []string) (c *config, err error) {
	for _, a := range arguments {
		if a == "-h" || a == "--help" {
			return nil, fmt.Errorf("--help is no option of a restart")
		}
	}
	checking = true
	defer func() {
		checking = false
		if r := recover(); r != nil {
			f, isUsage := r.(usageFailure)
			if !isUsage {
				panic(r)
			}
			c, err = nil, fmt.Errorf("%s", f.problem)
		}
	}()
	return parseArgs(arguments), nil
}

var negativeNumber = regexp.MustCompile(`^-\d+$|^-\d*\.\d+$`)

func looksLikeOption(arg string) bool {
	return len(arg) > 1 && arg[0] == '-' && !negativeNumber.MatchString(arg) && !strings.Contains(arg, " ")
}

// findOption resolves an argument as argparse does: the exact name, or an unambiguous prefix of one, with =VALUE or without.
func findOption(options []option, arg string) (found *option, value string, explicit bool) {
	if arg == "-h" {
		return &options[0], "", false
	}
	if !strings.HasPrefix(arg, "--") {
		return nil, "", false
	}
	name, value, explicit := strings.Cut(arg, "=")
	var candidates []string
	for i := range options {
		if options[i].name == name {
			return &options[i], value, explicit
		}
		if strings.HasPrefix(options[i].name, name) {
			found = &options[i]
			candidates = append(candidates, options[i].name)
		}
	}
	if len(candidates) > 1 {
		failUsage(options, fmt.Sprintf("ambiguous option: %s could match %s", arg, strings.Join(candidates, ", ")))
	}
	return found, value, explicit
}

func parseArgs(arguments []string) *config {
	c := &config{
		token:              "qa-token",
		repo:               "QaOwner/qa-mod",
		clientID:           "qa-client",
		deviceApproveAfter: 2,
		deviceInterval:     1,
		catalogRepo:        "QaMaintainer/darf-mods",
		vtKey:              "qa-vt-key",
		vtQueue:            1,
		uploadResultsText:  defaultUploadResults,
		vtMaxUpload:        33554432,
	}
	options := c.options()

	var extras []string
	for i := 0; i < len(arguments); i++ {
		arg := arguments[i]
		if arg == "--" {
			extras = append(extras, arguments[i+1:]...)
			break
		}
		opt, value, explicit := findOption(options, arg)
		if opt == nil {
			extras = append(extras, arg)
			continue
		}
		switch target := opt.target.(type) {
		case nil:
			fmt.Print(helpText(options))
			os.Exit(0)
		case *bool:
			if explicit {
				failUsage(options, fmt.Sprintf("argument %s: ignored explicit argument %s", opt.name, pyRepr(value)))
			}
			*target = true
		default:
			if !explicit {
				if i+1 == len(arguments) || looksLikeOption(arguments[i+1]) {
					failUsage(options, fmt.Sprintf("argument %s: expected one argument", opt.name))
				}
				i++
				value = arguments[i]
			}
			if text, isText := target.(*string); isText {
				*text = value
				break
			}
			number, err := strconv.Atoi(strip(value))
			if err != nil {
				failUsage(options, fmt.Sprintf("argument %s: invalid int value: %s", opt.name, pyRepr(value)))
			}
			*target.(*int) = number
		}
	}
	if len(extras) > 0 {
		failUsage(options, "unrecognized arguments: "+strings.Join(extras, " "))
	}

	c.validate(options)
	return c
}

func (c *config) validate(options []option) {
	for _, repository := range []struct{ option, value string }{{"--repo", c.repo}, {"--catalog-repo", c.catalogRepo}} {
		owner, name, _ := strings.Cut(repository.value, "/")
		if strings.Count(repository.value, "/") != 1 || owner == "" || name == "" {
			failUsage(options, repository.option+" takes owner/name")
		}
	}
	if c.login == "" {
		c.login, _, _ = strings.Cut(c.repo, "/")
	}
	if strings.Contains(c.login, "/") {
		failUsage(options, "--login takes a user name")
	}
	if strings.EqualFold(c.repo, c.catalogRepo) {
		failUsage(options, "--repo must not be the catalog")
	}
	if c.catalogSeed != "" {
		if info, err := os.Stat(c.catalogSeed); err != nil || !info.IsDir() {
			failUsage(options, "--catalog-seed takes a folder")
		}
	}
	if min(c.vtQueue, c.vtRate) < 0 || c.vtMaxUpload <= 0 {
		failUsage(options, "counts cannot be negative and --vt-max-upload must be positive")
	}
	value, valid := parseJSON([]byte(c.uploadResultsText))
	if !valid {
		failUsage(options, "--vt-upload-results: not valid JSON")
	}
	results, err := readVtResults(value, "--vt-upload-results")
	if err != nil {
		failUsage(options, err.Error())
	}
	c.vtUploadResults = results
}
