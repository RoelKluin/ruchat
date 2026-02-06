#!/usr/bin/perl
use strict;
use warnings;
use Getopt::Long;

my $func;
my $arg;
my $default;
my $file;
my $position = 'end';

GetOptions(
    "func=s" => \$func,
    "arg=s" => \$arg,
    "default=s" => \$default,
    "file=s" => \$file,
    "position=s" => \$position,
);

die "Usage: $0 --func <function_name> --arg <arg_name:type> --default <default_expr> [--file <file_path>] [--position <start|end>]\n" unless $func && $arg && $default;

die "Invalid --position, must be 'start' or 'end'\n" unless $position eq 'start' || $position eq 'end';

my ($arg_name, $arg_type) = map { s/^\s+|\s+$//g; $_ } split /:/, $arg, 2;

die "Invalid --arg format, expected 'name:type'\n" unless $arg_name && $arg_type;

# Find potential definition files
my @def_files = `rg --type rust -l "\\bfn\\s+\\Q$func\\E\\b\\s*\\("`;
chomp foreach @def_files;
my %unique_defs = map { $_ => 1 } @def_files;
@def_files = keys %unique_defs;

my $def_file;
if ($file) {
    if (exists $unique_defs{$file}) {
        $def_file = $file;
    } else {
        die "Specified file $file does not contain definition of $func\n";
    }
} else {
    if (@def_files == 0) {
        die "No definition found for $func\n";
    } elsif (@def_files > 1) {
        die "Multiple definitions found in files: @def_files\nPlease specify --file\n";
    } else {
        $def_file = $def_files[0];
    }
}

# Find all files containing the function name followed by '(' (calls and def)
my @files = `rg --type rust -l "\\b\\Q$func\\E\\b\\s*\\("`;
chomp foreach @files;
my %unique_files = map { $_ => 1 } @files;
@files = keys %unique_files;

# Modify each file
foreach my $f (@files) {
    local $/ = undef;
    open my $fh, '<', $f or die "Cannot open $f: $!";
    my $content = <$fh>;
    close $fh;

    # Modify the function definition if this is the def file
    if ($f eq $def_file) {
        $content =~ s{(fn\s+\Q$func\E\s*\()(.*?)(\))}{
            my ($pre, $params, $post) = ($1, $2, $3);
            my $new_arg = "$arg_name: $arg_type";
            my $new_params;
            if ($position eq 'end') {
                my $comma = ($params =~ /\S/ && $params !~ /,\s*$/) ? ', ' : '';
                $new_params = $params . $comma . $new_arg;
            } else { # start
                my $comma = ($params =~ /\S/) ? ', ' : '';
                $new_params = $new_arg . $comma . $params;
            }
            $pre . $new_params . $post
        }se;
    }

    # Modify the call sites
    $content =~ s{(?<!fn\s)\b\Q$func\E\b\s*\((.*?)(\))}{
        my ($params, $post) = ($1, $2);
        my $new_call_params;
        if ($position eq 'end') {
            my $comma = ($params !~ /\S/) ? '' : ($params =~ /,\s*$/) ? '' : ', ';
            $new_call_params = $params . $comma . $arg_name;
        } else { # start
            my $comma = ($params =~ /\S/) ? ', ' : '';
            $new_call_params = $arg_name . $comma . $params;
        }
        "{ let $arg_name = $default; \Q$func\E($new_call_params)$post }"
    }seg;

    # Write back the modified content
    open $fh, '>', $f or die "Cannot write to $f: $!";
    print $fh $content;
    close $fh;
}

# Test compilation
my $build_status = system('cargo build');
if ($build_status == 0) {
    my $commit_msg = "Add $arg_name: $arg_type argument to $func at $position and update call sites with default value";
    system('git add .');
    system("git commit -m \"$commit_msg\"");
} else {
    print "Compilation failed. Changes not committed.\n";
}
