use super::*;

fn check(args: &[&str]) -> Result<(), Refusal> {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    Parsed::new(&args).check()
}

#[test]
fn merge_approval_and_token_forms_are_refused_without_confusing_option_values() {
    for args in [
        vec!["pr", "merge", "42"],
        vec!["-Rowner/repo", "pr", "merge", "--auto"],
        vec!["pr", "--repo", "owner/repo", "merge"],
        vec!["pr", "review", "42", "-a"],
        vec!["pr", "review", "-ca", "-b", "notes"],
        vec!["pr", "review", "--approve=true"],
        vec!["pr", "review", "-a=1"],
        vec!["pr", "review", "-bnotes", "-a"],
        vec!["pr", "review", "--approve=false", "-a"],
        vec!["auth", "--hostname=example.test", "token"],
    ] {
        assert!(check(&args).is_err(), "{args:?}");
    }
    for args in [
        vec!["pr", "view", "42"],
        vec!["pr", "review", "--comment", "--body", "--approve"],
        vec!["pr", "review", "-c", "-ba"],
        vec!["pr", "review", "-r", "-Fapprove"],
        vec!["pr", "review", "--approve=false", "--comment"],
        vec!["pr", "review", "-a=false", "-c"],
        vec!["pr", "review", "-a", "--approve=false", "-c"],
        vec!["pr", "review", "--approve", "-a=0", "-c"],
        vec!["auth", "status"],
        vec!["--version"],
        vec!["pr", "create", "--body", "merge"],
    ] {
        assert!(check(&args).is_ok(), "{args:?}");
    }
}

#[test]
fn rest_endpoints_are_checked_with_url_query_and_field_spellings() {
    for endpoint in [
        "repos/{owner}/{repo}/pulls/42/merge",
        "/repos/o/r/pulls/42/reviews",
        "repos/o/r/pulls/42/reviews/7/events?x=y",
        "https://example.test/api/v3/repos/o/r/pulls/42/merge",
        "repos/o/r/merges",
        "repos/o/r/pulls/42/%6derge",
    ] {
        for before in [
            vec!["api", endpoint],
            vec!["api", "-XPUT", endpoint],
            vec!["api", "--field", "event=APPROVE", endpoint],
        ] {
            assert_eq!(
                check(&before).unwrap_err().code,
                "gh_api_merge_review",
                "{before:?}"
            );
        }
    }
    for args in [
        vec!["api", "repos/o/r/pulls/42"],
        vec!["api", "-f", "body=merge", "repos/o/r/issues/42/comments"],
        vec!["api", "repos/reviews/merge/issues"],
        vec!["api", "--jq", "merge", "user"],
        vec!["api", "repos/o/r/pulls/42/comments", "--input=-"],
    ] {
        assert!(check(&args).is_ok(), "{args:?}");
    }
}

#[test]
fn graphql_guards_mutations_and_opaque_queries_but_allows_read_queries() {
    for field in [
        "query=mutation { m: mergePullRequest(input: {}) { clientMutationId } }",
        "query=mutation { addPullRequestReview(input: {event: APPROVE}) { clientMutationId } }",
        "query=mutation { submitPullRequestReview(input: {}) { clientMutationId } }",
        "query=mutation { enablePullRequestAutoMerge(input: {}) { clientMutationId } }",
        "query=mutation { enqueuePullRequest(input: {}) { clientMutationId } }",
        "query=mutation { mergeBranch(input: {}) { clientMutationId } }",
    ] {
        assert_eq!(
            check(&["api", "graphql", "-f", field]).unwrap_err().code,
            "gh_api_merge_review"
        );
    }
    for args in [
        vec!["api", "graphql", "--input=-"],
        vec!["api", "graphql", "-Fquery=@query.graphql"],
        vec!["api", "graphql", "--field=query=@-"],
    ] {
        assert_eq!(
            check(&args).unwrap_err().code,
            "gh_graphql_body",
            "{args:?}"
        );
    }
    for query in [
        "query={ viewer { login } }",
        "query={ repository(name: \"mergePullRequest\", owner: \"o\") { id } }",
        "query={ viewer { login } } # mergePullRequest",
    ] {
        assert!(
            check(&["api", "graphql", "--raw-field", query]).is_ok(),
            "{query}"
        );
    }
}

#[test]
fn graphql_comments_and_block_strings_do_not_hide_following_mutations() {
    for newline in ["\r", "\n", "\r\n"] {
        let query = format!(
            "mutation {{ # comment{newline} mergePullRequest(input:{{}}){{clientMutationId}} }}"
        );
        assert!(guarded_graphql(&query));
    }
    assert!(guarded_graphql(
        r#"mutation($b:String="""Quote: " """){mergePullRequest(input:{commitBody:$b}){clientMutationId}}"#
    ));
    assert!(guarded_graphql(
        r#"mutation($b:String="""Escape: \""" more"""){mergePullRequest(input:{commitBody:$b}){clientMutationId}}"#
    ));
    assert!(!guarded_graphql(
        r#"{repository(name:"""a " mergePullRequest " b"""){id}}"#
    ));
    assert!(!guarded_graphql(
        r#"{repository(name:"""a \""" mergePullRequest b"""){id}}"#
    ));
    assert!(!guarded_graphql(
        r#"{repository(name:"a \" mergePullRequest"){id}}"#
    ));
}
